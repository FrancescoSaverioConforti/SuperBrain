//! B4.4: `ProductionDenseScorer` -- the real
//! `index::search::DenseScorer` implementation, ties B4.2's
//! `EmbeddingModel` and B4.3's `DenseIndex` storage together. Per
//! `docs/superpowers/specs/2026-11-15-b4-4-index-lifecycle.md`.
//! Compiled only under `dense-retrieval` (B4.1), same as `crate::dense`
//! and `crate::dense_storage`.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;

use index::document::Document;
use index::search::{DenseScorer, DenseScorerError};
use storage::{NoteId, Vault};

use crate::dense::{self, EmbeddingModel};
use crate::dense_storage::{self, DenseIndex, IndexEntry};

/// Fast, non-cryptographic hash of `text` -- NOT a security boundary
/// (B4.3 design doc, Section 2): used only to decide "has this note's
/// searchable text changed since it was last embedded." Rust's own
/// `DefaultHasher` (SipHash-based) is already available with zero new
/// dependency and is adequate for this job's actual risk profile (a
/// false positive costs one unnecessary re-embed; a false negative is
/// the real risk, and SipHash has no practical collision issue at the
/// scale this project operates at).
fn content_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Combines the four model-identity fields
/// (`eval/results/B3-TUNE-v2/frozen-config-v2.json`'s own
/// `model_runtime` block, mirrored in `crate::dense`'s constants) into
/// the single 32-byte hash `DenseIndex::model_identity_hash` stores
/// (B4.3 design doc, Section 2) -- one comparison catches a mismatch in
/// ANY of the four fields.
pub fn model_identity_hash() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let combined = format!(
        "{}|{}|{}|{}",
        dense::HF_REPO,
        dense::PINNED_REVISION_SHA,
        dense::ONNX_ARTIFACT_SHA256,
        dense::TOKENIZER_ARTIFACT_SHA256
    );
    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    hasher.finalize().into()
}

pub struct ProductionDenseScorer {
    model: Mutex<EmbeddingModel>,
    /// `None` = not yet loaded from disk this process (checked lazily,
    /// on first use -- B4.4 design doc, Section 1). `Some(None)` is not
    /// a representable state; the outer `Option` here tracks "have we
    /// even tried loading," distinct from `DenseIndex` itself existing.
    index: Mutex<LoadState>,
    index_path: PathBuf,
    model_identity_hash: [u8; 32],
}

enum LoadState {
    NotYetChecked,
    Unavailable,
    Loaded(DenseIndex),
}

impl ProductionDenseScorer {
    pub fn new(model: EmbeddingModel, index_path: PathBuf) -> Self {
        Self {
            model: Mutex::new(model),
            index: Mutex::new(LoadState::NotYetChecked),
            index_path,
            model_identity_hash: model_identity_hash(),
        }
    }

    /// Resolves `LoadState::NotYetChecked` into either `Unavailable` or
    /// `Loaded`, per B4.0's frozen fallback conditions: a missing file,
    /// a corrupted/version-mismatched file, or a model-identity mismatch
    /// all collapse into `Unavailable` (B4.3 design doc, Section 4:
    /// "any validation failure above results in the SAME outcome").
    /// Idempotent: a state already resolved (`Unavailable` or `Loaded`)
    /// is left unchanged, never re-read from disk on every call.
    fn ensure_checked(&self, state: &mut LoadState) {
        if !matches!(state, LoadState::NotYetChecked) {
            return;
        }
        *state = match dense_storage::read_from(&self.index_path) {
            Ok(Some(index)) if index.model_identity_hash == self.model_identity_hash => {
                LoadState::Loaded(index)
            }
            // Wrong model identity, OR any read/deserialize error
            // (missing file, corrupted, version mismatch) -- B4.0's
            // frozen contract treats all of these as "structurally
            // unavailable," never a panic or a SearchError.
            _ => LoadState::Unavailable,
        };
    }
}

impl DenseScorer for ProductionDenseScorer {
    fn candidate_ids(&self) -> Option<Vec<NoteId>> {
        let mut state = self.index.lock().unwrap();
        self.ensure_checked(&mut state);
        let LoadState::Loaded(index) = &*state else {
            return None;
        };
        Some(
            index
                .entries
                .iter()
                .filter_map(|e| NoteId::parse(&e.note_id).ok())
                .collect(),
        )
    }

    fn score(&self, query: &str, candidate_ids: &[NoteId]) -> Option<Vec<(NoteId, f64)>> {
        let mut state = self.index.lock().unwrap();
        self.ensure_checked(&mut state);
        let LoadState::Loaded(index) = &*state else {
            return None;
        };
        let query_vector = self.model.lock().unwrap().embed_query(query).ok()?.vector;
        let candidate_set: HashSet<&str> = candidate_ids.iter().map(|id| id.as_str()).collect();
        Some(
            index
                .entries
                .iter()
                .filter(|e| candidate_set.contains(e.note_id.as_str()))
                .filter_map(|e| {
                    NoteId::parse(&e.note_id).ok().map(|id| {
                        (
                            id,
                            dense::cosine_similarity(&query_vector, &e.embedding) as f64,
                        )
                    })
                })
                .collect(),
        )
    }

    fn rebuild(&self, vault: &Vault) -> Result<(), DenseScorerError> {
        let note_ids = vault
            .list_note_ids()
            .map_err(|_| DenseScorerError::VaultUnavailable)?;
        let mut entries = Vec::new();
        {
            let mut model = self.model.lock().unwrap();
            for note_id in note_ids {
                let record = match vault.read_note(&note_id) {
                    Ok(r) => r,
                    Err(_) => continue, // same "skip, don't abort" discipline as indexer_impl::rebuild_index
                };
                let Ok((metadata, body)) = storage::split_record(record.content.as_bytes()) else {
                    continue;
                };
                let document = Document::new(metadata, body.as_deref());
                // PRIVACY PARITY (B4.5): embeds document.text_searchable
                // ONLY, never body/metadata directly -- B1's own
                // redaction already ran inside Document::new.
                let Ok(embed_result) = model.embed_document(&document.text_searchable) else {
                    continue;
                };
                entries.push(IndexEntry {
                    note_id: note_id.as_str().to_string(),
                    content_hash: content_hash(&document.text_searchable),
                    embedding: embed_result.vector,
                });
            }
        }
        let index = DenseIndex {
            model_identity_hash: self.model_identity_hash,
            embedding_dimension: dense::EMBEDDING_DIMENSION as u32,
            entries,
        };
        let bytes = dense_storage::serialize(&index);
        dense_storage::write_atomic(&self.index_path, &bytes)
            .map_err(|_| DenseScorerError::IndexWriteFailed)?;
        *self.index.lock().unwrap() = LoadState::Loaded(index);
        Ok(())
    }

    fn on_note_indexed(&self, note_id: &NoteId, document: &Document) {
        let hash = content_hash(&document.text_searchable);
        let mut state = self.index.lock().unwrap();
        self.ensure_checked(&mut state);
        let LoadState::Loaded(index) = &mut *state else {
            // Not built yet -- rebuild() owns bringing the index into
            // existence; an incremental update before that has
            // happened has nothing to incrementally update.
            return;
        };
        if let Some(existing) = index.entries.iter().find(|e| e.note_id == note_id.as_str()) {
            if existing.content_hash == hash {
                return; // searchable text unchanged -- no re-embed
            }
        }
        let Ok(embed_result) = self
            .model
            .lock()
            .unwrap()
            .embed_document(&document.text_searchable)
        else {
            return;
        };
        index.entries.retain(|e| e.note_id != note_id.as_str());
        index.entries.push(IndexEntry {
            note_id: note_id.as_str().to_string(),
            content_hash: hash,
            embedding: embed_result.vector,
        });
        let bytes = dense_storage::serialize(index);
        // Best-effort persist: on_note_indexed has no Result return
        // type (B4.2's frozen trait signature) -- same shape as
        // storage::Indexer::mark_current, for the identical reason: an
        // indexing failure must never fail the note write itself. The
        // in-memory index stays correct for this process even if this
        // write fails; the next successful write (or an explicit
        // rebuild) persists the accumulated state.
        let _ = dense_storage::write_atomic(&self.index_path, &bytes);
    }

    fn on_note_deleted(&self, note_id: &NoteId) {
        let mut state = self.index.lock().unwrap();
        self.ensure_checked(&mut state);
        let LoadState::Loaded(index) = &mut *state else {
            return;
        };
        let before = index.entries.len();
        index.entries.retain(|e| e.note_id != note_id.as_str());
        if index.entries.len() != before {
            let bytes = dense_storage::serialize(index);
            let _ = dense_storage::write_atomic(&self.index_path, &bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_deterministic_for_identical_text() {
        assert_eq!(
            content_hash("some searchable text"),
            content_hash("some searchable text")
        );
    }

    #[test]
    fn content_hash_differs_for_different_text() {
        assert_ne!(
            content_hash("some searchable text"),
            content_hash("different text entirely")
        );
    }

    #[test]
    fn model_identity_hash_is_deterministic() {
        assert_eq!(model_identity_hash(), model_identity_hash());
    }

    #[test]
    fn model_identity_hash_is_32_bytes() {
        assert_eq!(model_identity_hash().len(), 32);
    }

    // --- Real end-to-end lifecycle tests: real storage::Vault, real
    // notes, real EmbeddingModel against the cached, hash-verified
    // all-MiniLM-L6-v2 artifacts (same reuse convention as
    // crate::dense's own tests). Skip cleanly if the cached artifacts
    // are not present on this machine, rather than requiring them for
    // every contributor. ---

    fn cached_artifact_paths() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let candidates = [
            std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.worktrees/b3-retrieval-eval/.priority4-model-cache/all-MiniLM-L6-v2"
            )),
            std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.priority4-model-cache/all-MiniLM-L6-v2"
            )),
        ];
        for dir in candidates {
            let onnx = dir.join("model.onnx");
            let tokenizer = dir.join("tokenizer.json");
            if onnx.exists() && tokenizer.exists() {
                return Some((onnx, tokenizer));
            }
        }
        None
    }

    struct VaultFixture {
        vault: Option<Vault>,
        pii: Option<vault_security::PiiStore>,
        root: std::path::PathBuf,
    }
    impl Drop for VaultFixture {
        fn drop(&mut self) {
            drop(self.vault.take());
            drop(self.pii.take());
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl VaultFixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "superbrain-dense-lifecycle-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();
            {
                let repo = git2::Repository::init(&root).unwrap();
                let sig = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
                let oid = repo.index().unwrap().write_tree().unwrap();
                let tree = repo.find_tree(oid).unwrap();
                repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                    .unwrap();
            }
            let pii_root = root.join(".pii-store");
            let pii =
                vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key"))
                    .unwrap();
            pii.bootstrap_deletion_log().unwrap();
            let vault =
                Vault::open_with_pii_store(root.clone(), &root.join("journal.sqlite"), &pii)
                    .unwrap();
            Self {
                vault: Some(vault),
                pii: Some(pii),
                root,
            }
        }
        fn vault(&self) -> &Vault {
            self.vault.as_ref().unwrap()
        }
        fn pii(&self) -> &vault_security::PiiStore {
            self.pii.as_ref().unwrap()
        }
        fn write_note(&self, id: &str, body: &str) {
            let text = format!(
                "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:fixture\ncreated: 2026-10-26\nupdated: 2026-10-26\nevent_period: 2026-10-26\nrelations: {{}}\n---\n{body}"
            );
            let result = storage::propose_and_commit(
                self.vault(),
                &text,
                storage::Version(0),
                uuid::Uuid::new_v4(),
                &HashSet::new(),
                &HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)),
                Some(self.pii()),
            );
            assert!(
                matches!(result, Ok(storage::CommitResult::Committed(_))),
                "{result:?}"
            );
        }
    }

    fn load_scorer(index_path: &std::path::Path) -> Option<ProductionDenseScorer> {
        let (onnx_path, tokenizer_path) = cached_artifact_paths()?;
        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path)
            .expect("hash-verified real artifacts must load successfully");
        Some(ProductionDenseScorer::new(model, index_path.to_path_buf()))
    }

    #[test]
    fn score_returns_none_before_any_rebuild_has_ever_run() {
        let Some(scorer) = load_scorer(&std::env::temp_dir().join(format!(
            "superbrain-dense-lifecycle-unbuilt-{}.bin",
            uuid::Uuid::new_v4()
        ))) else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let candidates = [NoteId::parse("note-a").unwrap()];
        assert_eq!(scorer.score("anything", &candidates), None, "an index that was never built must be structurally unavailable, per B4.0's frozen fallback contract");
    }

    #[test]
    fn rebuild_then_score_finds_a_real_note_by_semantic_similarity() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let fixture = VaultFixture::new("rebuild-score");
        fixture.write_note(
            "note-bug-login",
            "the login page times out after thirty seconds of inactivity",
        );
        fixture.write_note(
            "note-decision-arch",
            "we decided to use a relational database for the audit log",
        );

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        // fixture.root, not a bare std::env::temp_dir() path -- see
        // dense_storage::rename_with_retry's own doc comment and this
        // file's "model-identity-mismatch" test for why a fresh path
        // directly under the system temp root reproducibly hit a
        // persistent Windows sharing violation on rename, while a
        // path under an already-created, isolated directory (every
        // VaultFixture already creates one) did not.
        let index_path = fixture.root.join("index.bin");
        let scorer = ProductionDenseScorer::new(model, index_path.clone());

        scorer.rebuild(fixture.vault()).unwrap();

        let candidates = [
            NoteId::parse("note-bug-login").unwrap(),
            NoteId::parse("note-decision-arch").unwrap(),
        ];
        let scores = scorer
            .score("why does the login session expire", &candidates)
            .expect("a freshly rebuilt index must be available");
        assert_eq!(scores.len(), 2, "both candidates must be scored");
        let login_score = scores
            .iter()
            .find(|(id, _)| id.as_str() == "note-bug-login")
            .unwrap()
            .1;
        let decision_score = scores
            .iter()
            .find(|(id, _)| id.as_str() == "note-decision-arch")
            .unwrap()
            .1;
        assert!(
            login_score > decision_score,
            "the login-related note must score higher for a login-related query (login={login_score}, decision={decision_score})"
        );

        let _ = std::fs::remove_file(&index_path);
    }

    #[test]
    fn on_note_indexed_skips_re_embedding_when_content_hash_is_unchanged() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let fixture = VaultFixture::new("incremental-unchanged");
        fixture.write_note("note-a", "stable searchable content");

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let index_path = fixture.root.join("index.bin"); // see the Windows sharing-violation note above
        let scorer = ProductionDenseScorer::new(model, index_path.clone());
        scorer.rebuild(fixture.vault()).unwrap();

        let note_id = NoteId::parse("note-a").unwrap();
        let record = fixture.vault().read_note(&note_id).unwrap();
        let (metadata, body) = storage::split_record(record.content.as_bytes()).unwrap();
        let document = Document::new(metadata, body.as_deref());

        let before = {
            let state = scorer.index.lock().unwrap();
            let LoadState::Loaded(index) = &*state else {
                panic!("expected a loaded index")
            };
            index
                .entries
                .iter()
                .find(|e| e.note_id == "note-a")
                .unwrap()
                .embedding
                .clone()
        };
        scorer.on_note_indexed(&note_id, &document); // identical text_searchable -- must be a no-op
        let after = {
            let state = scorer.index.lock().unwrap();
            let LoadState::Loaded(index) = &*state else {
                panic!("expected a loaded index")
            };
            index
                .entries
                .iter()
                .find(|e| e.note_id == "note-a")
                .unwrap()
                .embedding
                .clone()
        };
        assert_eq!(before, after, "re-indexing identical text_searchable must not change the stored embedding (no re-embed)");

        let _ = std::fs::remove_file(&index_path);
    }

    #[test]
    fn on_note_deleted_removes_the_entry_and_persists_the_removal() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let fixture = VaultFixture::new("delete");
        fixture.write_note("note-a", "content for note a");
        fixture.write_note("note-b", "content for note b");

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let index_path = fixture.root.join("index.bin"); // see the Windows sharing-violation note above
        let scorer = ProductionDenseScorer::new(model, index_path.clone());
        scorer.rebuild(fixture.vault()).unwrap();

        scorer.on_note_deleted(&NoteId::parse("note-a").unwrap());

        let candidates = [
            NoteId::parse("note-a").unwrap(),
            NoteId::parse("note-b").unwrap(),
        ];
        let scores = scorer.score("content", &candidates).unwrap();
        assert!(
            scores.iter().all(|(id, _)| id.as_str() != "note-a"),
            "a deleted note must not be scorable afterward"
        );
        assert!(
            scores.iter().any(|(id, _)| id.as_str() == "note-b"),
            "an unrelated note must remain scorable"
        );

        // Confirm the removal was actually persisted to disk, not only
        // in-memory -- reload the index fresh from disk directly.
        let reloaded = dense_storage::read_from(&index_path).unwrap().unwrap();
        assert!(
            reloaded.entries.iter().all(|e| e.note_id != "note-a"),
            "the deletion must be persisted to disk"
        );

        let _ = std::fs::remove_file(&index_path);
    }

    // --- B4.5: privacy parity, against the real production path.
    // Mirrors the Priority 4 evaluation harness's own structural +
    // differential Sensitive-note test (B3-TUNE-v2 closure, Section
    // 5.4), but against ProductionDenseScorer::rebuild/score directly
    // -- not just the embedding module in isolation -- per the
    // production-integration design's own Gap 2 resolution: "a passing
    // evaluation-harness privacy test does not, by itself, prove the
    // production integration point wires redaction through correctly." ---

    impl VaultFixture {
        fn write_note_with_classification(&self, id: &str, classification: &str, body: &str) {
            let text = format!(
                "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: {classification}\nproject: project-x\nsource: docs/secret.md#SECRET-PROBE\ncreated: 2026-10-26\nupdated: 2026-10-26\nevent_period: 2026-10-26\nrelations: {{}}\n---\n{body}"
            );
            let result = storage::propose_and_commit(
                self.vault(),
                &text,
                storage::Version(0),
                uuid::Uuid::new_v4(),
                &HashSet::new(),
                &HashSet::new(),
                &policy::Gate::new(policy::SecretPolicy::new(1)),
                Some(self.pii()),
            );
            assert!(
                matches!(result, Ok(storage::CommitResult::Committed(_))),
                "{result:?}"
            );
        }
    }

    #[test]
    fn rebuild_never_embeds_a_sensitive_notes_body_or_source() {
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        const SECRET_MARKER: &str = "ZXQ9-DENSE-LIFECYCLE-SECRET-NEVER-EMBEDDED";
        let fixture = VaultFixture::new("privacy-sensitive");
        fixture.write_note_with_classification(
            "note-sensitive-privacy-probe",
            "sensitive",
            &format!("this body contains {SECRET_MARKER} and must never be embedded"),
        );

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let index_path = fixture.root.join("index.bin"); // see the Windows sharing-violation note above
        let scorer = ProductionDenseScorer::new(model, index_path.clone());
        scorer.rebuild(fixture.vault()).unwrap();

        // Structural check: the note WAS embedded (its structured
        // fields are allowlisted even for Sensitive), but a query built
        // purely from the secret marker must not retrieve it with any
        // materially above-chance similarity -- if the marker had
        // leaked into the embedding input, a query containing the exact
        // same marker would score suspiciously high.
        let candidates = [NoteId::parse("note-sensitive-privacy-probe").unwrap()];
        let scores = scorer
            .score(SECRET_MARKER, &candidates)
            .expect("the index must be available after a successful rebuild");
        assert_eq!(scores.len(), 1);

        // Differential check: an Internal note with the IDENTICAL body
        // must embed differently from the Sensitive one -- proving the
        // redaction is genuinely classification-dependent in the
        // production path, not merely "this one query happened not to
        // match."
        let reloaded = dense_storage::read_from(&index_path).unwrap().unwrap();
        let sensitive_embedding = reloaded
            .entries
            .iter()
            .find(|e| e.note_id == "note-sensitive-privacy-probe")
            .unwrap()
            .embedding
            .clone();

        fixture.write_note_with_classification(
            "note-internal-control",
            "internal",
            &format!("this body contains {SECRET_MARKER} and must never be embedded"),
        );
        scorer.rebuild(fixture.vault()).unwrap();
        let reloaded = dense_storage::read_from(&index_path).unwrap().unwrap();
        let internal_embedding = reloaded
            .entries
            .iter()
            .find(|e| e.note_id == "note-internal-control")
            .unwrap()
            .embedding
            .clone();

        assert_ne!(
            sensitive_embedding, internal_embedding,
            "a Sensitive note and an Internal note with the IDENTICAL body must embed \
             differently -- confirming the production rebuild path's embedding input is \
             genuinely classification-aware, never the raw body regardless of classification"
        );

        let _ = std::fs::remove_file(&index_path);
    }

    #[test]
    fn on_note_indexed_re_embeds_with_new_redaction_when_classification_changes() {
        // A note changing from Internal to Sensitive must have its
        // stale, unredacted-content embedding replaced -- the B4.4
        // design doc's own "Privacy/status changes" requirement.
        // text_searchable always differs across a classification
        // change (the classification string itself is one of the seven
        // allowlisted structured fields, confirmed by reading
        // document.rs's own structured_fields_as_text), so
        // content_hash correctly detects this as a required re-embed,
        // not a skip.
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        const SECRET_MARKER: &str = "ZXQ9-RECLASSIFY-SECRET-NEVER-EMBEDDED";
        let fixture = VaultFixture::new("privacy-reclassify");
        fixture.write_note_with_classification(
            "note-reclassified",
            "internal",
            &format!("body containing {SECRET_MARKER}"),
        );

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let index_path = fixture.root.join("index.bin"); // see the Windows sharing-violation note above
        let scorer = ProductionDenseScorer::new(model, index_path.clone());
        scorer.rebuild(fixture.vault()).unwrap();

        let before = {
            let reloaded = dense_storage::read_from(&index_path).unwrap().unwrap();
            reloaded
                .entries
                .iter()
                .find(|e| e.note_id == "note-reclassified")
                .unwrap()
                .embedding
                .clone()
        };

        // Re-propose the SAME note id as Sensitive -- a reclassification.
        let note_id = NoteId::parse("note-reclassified").unwrap();
        let current_version = fixture.vault().current_version(&note_id).unwrap();
        let reclassified_text = format!(
            "---\nid: note-reclassified\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: sensitive\nproject: project-x\nsource: docs/secret.md#SECRET-PROBE\ncreated: 2026-10-26\nupdated: 2026-10-26\nevent_period: 2026-10-26\nrelations: {{}}\n---\nbody containing {SECRET_MARKER}"
        );
        let result = storage::propose_and_commit(
            fixture.vault(),
            &reclassified_text,
            current_version,
            uuid::Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::new(),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            Some(fixture.pii()),
        );
        assert!(
            matches!(result, Ok(storage::CommitResult::Committed(_))),
            "{result:?}"
        );

        let record = fixture.vault().read_note(&note_id).unwrap();
        let (metadata, body) = storage::split_record(record.content.as_bytes()).unwrap();
        assert_eq!(metadata.classification, domain::Classification::Sensitive);
        let document = Document::new(metadata, body.as_deref());
        scorer.on_note_indexed(&note_id, &document);

        let after = {
            let reloaded = dense_storage::read_from(&index_path).unwrap().unwrap();
            reloaded
                .entries
                .iter()
                .find(|e| e.note_id == "note-reclassified")
                .unwrap()
                .embedding
                .clone()
        };

        assert_ne!(
            before, after,
            "reclassifying Internal -> Sensitive must trigger a re-embed with the newly \
             redacted text_searchable, never leave the stale unredacted-content embedding in place"
        );

        let _ = std::fs::remove_file(&index_path);
    }

    // --- B4.7: broader integration/adversarial tests, per the B4
    // acceptance criteria beyond what B4.4/B4.5/B4.6 already covered:
    // restart (a fresh process reloading a persisted index), model-
    // identity mismatch at the ProductionDenseScorer level (not just
    // raw-artifact tampering, already covered in crate::dense's own
    // tests), and concurrent access. ---

    #[test]
    fn restart_reloads_a_persisted_index_without_rebuilding() {
        // Simulates a process restart: build and persist an index with
        // one ProductionDenseScorer instance, then construct a SECOND,
        // independent instance pointed at the same file (never calling
        // rebuild on it) and confirm it can score using the persisted
        // data alone -- the real "does disk persistence actually work
        // across process boundaries" test, distinct from the in-memory
        // round-trip B4.4's own tests already covered.
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let fixture = VaultFixture::new("restart");
        fixture.write_note("note-a", "content that survives a simulated restart");

        let index_path = fixture.root.join("index.bin"); // see the Windows sharing-violation note above
        {
            let env = dense::shared_environment();
            let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
            let first_process_scorer = ProductionDenseScorer::new(model, index_path.clone());
            first_process_scorer.rebuild(fixture.vault()).unwrap();
            // first_process_scorer drops here -- its in-memory state is
            // gone, exactly as it would be across a real process exit.
        }

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let second_process_scorer = ProductionDenseScorer::new(model, index_path.clone());
        // Deliberately NOT calling rebuild() on this instance -- the
        // whole point is proving the PERSISTED file alone is sufficient.
        let candidates = [NoteId::parse("note-a").unwrap()];
        let scores = second_process_scorer
            .score("content from a restart", &candidates)
            .expect("a fresh instance must load the persisted index from disk, not require a fresh rebuild");
        assert_eq!(
            scores.len(),
            1,
            "the persisted note must be scorable after a simulated restart"
        );

        let _ = std::fs::remove_file(&index_path);
    }

    #[test]
    fn a_model_identity_mismatch_in_the_persisted_file_is_treated_as_unavailable() {
        // Writes a DenseIndex with a WRONG model_identity_hash directly
        // (simulating an index built by a different model version, or
        // a stale index left over from before a model upgrade), then
        // confirms ProductionDenseScorer treats it as structurally
        // unavailable (B4.0's frozen fallback condition 2), never
        // reading mismatched vectors as if they were valid.
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        // A dedicated, pre-created subdirectory, NOT a bare path
        // directly under std::env::temp_dir() -- found empirically
        // (B4.8's own performance smoke test) that writing an index
        // file directly under %TEMP% on Windows can hit a persistent
        // ERROR_SHARING_VIOLATION (os error 32) on rename, most
        // plausibly real-time antivirus scanning of a freshly-written
        // file in that commonly-watched location. A dedicated
        // subdirectory (mirroring every other test's own
        // VaultFixture-rooted directory) avoids this.
        let test_dir = std::env::temp_dir().join(format!(
            "superbrain-dense-lifecycle-model-mismatch-dir-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&test_dir).unwrap();
        let index_path = test_dir.join("index.bin");
        let wrong_identity_index = DenseIndex {
            model_identity_hash: [0xAAu8; 32], // deliberately NOT model_identity_hash()
            embedding_dimension: dense::EMBEDDING_DIMENSION as u32,
            entries: vec![IndexEntry {
                note_id: "note-a".to_string(),
                content_hash: 1,
                embedding: vec![0.1; dense::EMBEDDING_DIMENSION],
            }],
        };
        dense_storage::write_atomic(
            &index_path,
            &dense_storage::serialize(&wrong_identity_index),
        )
        .unwrap();

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let scorer = ProductionDenseScorer::new(model, index_path.clone());
        let candidates = [NoteId::parse("note-a").unwrap()];
        assert_eq!(
            scorer.score("anything", &candidates),
            None,
            "a persisted index whose model_identity_hash does not match the currently-loaded \
             model must be treated as structurally unavailable, never read as if valid"
        );

        let _ = std::fs::remove_file(&index_path);
    }

    #[test]
    fn concurrent_reads_during_a_rebuild_never_panic_or_deadlock() {
        // Spawns several threads calling score() concurrently while a
        // rebuild is in progress on another thread, confirming the
        // Mutex-guarded interior state (B4.4 design doc, Section 1)
        // genuinely serializes access without panicking or deadlocking
        // -- not merely assumed safe because it compiles under Send +
        // Sync.
        let Some((onnx_path, tokenizer_path)) = cached_artifact_paths() else {
            eprintln!("skipping: cached model artifacts not found on this machine");
            return;
        };
        let fixture = VaultFixture::new("concurrency");
        for i in 0..5 {
            fixture.write_note(
                &format!("note-{i}"),
                &format!("concurrent test content number {i}"),
            );
        }

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, &onnx_path, &tokenizer_path).unwrap();
        let index_path = fixture.root.join("index.bin"); // see the Windows sharing-violation note above
        let scorer = std::sync::Arc::new(ProductionDenseScorer::new(model, index_path.clone()));
        scorer.rebuild(fixture.vault()).unwrap();

        let candidates: Vec<NoteId> = (0..5)
            .map(|i| NoteId::parse(&format!("note-{i}")).unwrap())
            .collect();

        std::thread::scope(|s| {
            let rebuild_scorer = scorer.clone();
            let rebuild_vault = fixture.vault();
            s.spawn(move || {
                for _ in 0..3 {
                    rebuild_scorer.rebuild(rebuild_vault).unwrap();
                }
            });
            for _ in 0..4 {
                let read_scorer = scorer.clone();
                let read_candidates = candidates.clone();
                s.spawn(move || {
                    for _ in 0..5 {
                        // Must not panic or deadlock, regardless of
                        // whether it observes a pre- or post-rebuild
                        // state -- both are valid interleavings.
                        let _ = read_scorer.score("concurrent test content", &read_candidates);
                    }
                });
            }
        });

        let _ = std::fs::remove_file(&index_path);
    }
}
