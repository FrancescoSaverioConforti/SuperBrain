# B4.4 — Index Lifecycle: `ProductionDenseScorer`

**Status: design frozen before implementation.** Ties B4.2's
`EmbeddingModel` and B4.3's `DenseIndex` storage format together into
the real `index::search::DenseScorer` implementation.

## 1. Interior mutability

`DenseScorer`'s trait methods all take `&self` (B4.2's frozen
contract, matching `storage::Indexer`'s own `&self`-only shape). The
real implementation needs mutable state (the loaded `EmbeddingModel`
needs `&mut self` for `embed_*`, per B4.2; the in-memory `DenseIndex`
changes on every write). **Decision: `Mutex<...>` around both**,
mirroring `index::indexer_impl::RealIndexer`'s own exact pattern
(`data: Mutex<IndexData>`) — this project already has a precedent for
"a `&self`-only public trait method wrapping `Mutex`-guarded internal
state," not a new pattern invented for this module.

```rust
pub struct ProductionDenseScorer {
    model: Mutex<dense::EmbeddingModel>,
    index: Mutex<Option<dense_storage::DenseIndex>>,
    index_path: PathBuf,
    model_identity_hash: [u8; 32],
}
```

`index: Mutex<Option<DenseIndex>>` — `None` means "not yet loaded from
disk this process" (checked once, lazily, on first use, not eagerly at
construction — mirrors B4.0's own "lazy load" candidate approach for
the model itself). `Some(index)` is the in-memory, authoritative
current state; every write goes through this in-memory copy AND is
persisted to disk (B4.3's `write_atomic`) before the call returns —
there is no separate "dirty, not yet flushed" state, keeping the
on-disk file always consistent with what the last successful call saw
(simpler than a write-behind cache, and consistent with this project's
own "fail closed, never silently diverge from disk" discipline
throughout `storage`).

## 2. `rebuild`

```rust
fn rebuild(&self, vault: &Vault) -> Result<(), DenseScorerError> {
    let note_ids = vault.list_note_ids().map_err(|_| DenseScorerError::VaultUnavailable)?;
    let mut entries = Vec::new();
    let mut model = self.model.lock().unwrap();
    for note_id in note_ids {
        let record = match vault.read_note(&note_id) {
            Ok(r) => r,
            Err(_) => continue, // same "skip, don't abort" discipline as indexer_impl::rebuild_index
        };
        let Ok((metadata, body)) = storage::split_record(record.content.as_bytes()) else { continue };
        let document = index::document::Document::new(metadata, body.as_deref());
        // PRIVACY PARITY (B4.5): embeds document.text_searchable ONLY,
        // never body/metadata directly -- same type-level enforcement
        // DenseScorer::on_note_indexed's own signature already has.
        let Ok(embed_result) = model.embed_document(&document.text_searchable) else { continue };
        entries.push(dense_storage::IndexEntry {
            note_id: note_id.as_str().to_string(),
            content_hash: content_hash(&document.text_searchable),
            embedding: embed_result.vector,
        });
    }
    let index = dense_storage::DenseIndex {
        model_identity_hash: self.model_identity_hash,
        embedding_dimension: dense::EMBEDDING_DIMENSION as u32,
        entries,
    };
    let bytes = dense_storage::serialize(&index);
    dense_storage::write_atomic(&self.index_path, &bytes).map_err(|_| DenseScorerError::IndexWriteFailed)?;
    *self.index.lock().unwrap() = Some(index);
    Ok(())
}
```

Mirrors `index::indexer_impl::rebuild_index`'s own cold-start contract
exactly: called outside any writer's lock, enumerates via
`list_note_ids` + `read_note` per id, skips (not aborts on) an
individual unreadable/unparseable note. **Difference from the lexical
rebuild:** this one also WRITES the result to disk atomically (B4.3)
and updates the in-memory cache — the lexical `rebuild_index` only
populates in-memory structures, since B1's lexical indexes are
rebuilt fresh on every process start and never persisted; the dense
index IS persisted specifically because re-embedding every note on
every process start would reintroduce the "155ms model-load + N ×
embedding-inference-time" cost the whole point of persistence exists
to avoid.

## 3. `content_hash`

A fast, non-cryptographic 64-bit hash of `text_searchable` — Rust's
own `std::hash::Hasher` (`DefaultHasher`, SipHash-based) is already
available with zero new dependency, and is adequate here precisely
because this is NOT a security boundary (B4.3 design doc, Section 2):
it only needs to detect "this note's searchable text probably
changed," where a false positive (unnecessary re-embed) is cheap and
a false negative (missed re-embed) is the real risk to avoid — SipHash
has no known practical collision issues at this scale (thousands of
notes), so no justification for a cryptographic hash here.

## 4. `on_note_indexed` — incremental update

```rust
fn on_note_indexed(&self, note_id: &NoteId, document: &Document) {
    let hash = content_hash(&document.text_searchable);
    let mut index_guard = self.index.lock().unwrap();
    let Some(index) = index_guard.as_mut() else { return }; // not built yet -- rebuild() owns that
    if let Some(existing) = index.entries.iter().find(|e| e.note_id == note_id.as_str()) {
        if existing.content_hash == hash {
            return; // searchable text unchanged -- no re-embed (B4's own "incremental update" rule)
        }
    }
    let Ok(embed_result) = self.model.lock().unwrap().embed_document(&document.text_searchable) else { return };
    index.entries.retain(|e| e.note_id != note_id.as_str());
    index.entries.push(IndexEntry { note_id: note_id.as_str().to_string(), content_hash: hash, embedding: embed_result.vector });
    let bytes = dense_storage::serialize(index);
    let _ = dense_storage::write_atomic(&self.index_path, &bytes); // best-effort persist; in-memory stays authoritative for this process even if the disk write fails transiently
}
```

**Explicit design choice: `on_note_indexed` is a no-op if the index
has never been built** (`index_guard.as_mut()` returns `None`) —
incremental update only makes sense once there IS something to
incrementally update; the first call that matters is `rebuild`, not
this method. This mirrors `RealIndexer::index_document`'s own
"version/note not yet observed → return early" discipline.

**Explicit design choice: a failed disk write here does not fail the
call** (`let _ = write_atomic(...)`) — because `on_note_indexed` has
no `Result` return type at all (B4.2's frozen trait signature), and
because B1's own `Indexer::mark_current` has the identical shape
(`fn mark_current(&self, ...)`, no `Result`) for the identical reason:
an indexing failure must never fail the note write itself. The
in-memory index remains correct for this process even if the disk
write failed; the NEXT successful write (or an explicit `rebuild`)
will persist the accumulated state. This is an accepted, named
limitation (B4's closure doc), not silently ignored.

## 5. `on_note_deleted`

```rust
fn on_note_deleted(&self, note_id: &NoteId) {
    let mut index_guard = self.index.lock().unwrap();
    let Some(index) = index_guard.as_mut() else { return };
    let before = index.entries.len();
    index.entries.retain(|e| e.note_id != note_id.as_str());
    if index.entries.len() != before {
        let bytes = dense_storage::serialize(index);
        let _ = dense_storage::write_atomic(&self.index_path, &bytes);
    }
}
```

Only writes to disk if something was actually removed (avoids a
no-op disk write for a delete call on a note that was never in the
dense index in the first place — e.g. it was never successfully
embedded).

## 6. `score`

```rust
fn score(&self, query: &str, candidate_ids: &[NoteId]) -> Option<Vec<(NoteId, f64)>> {
    let index_guard = self.index.lock().unwrap();
    let index = self.ensure_loaded(&index_guard)?; // lazy-load from disk on first use; None if missing/corrupted/mismatched (B4.0 fallback conditions 1-2)
    let mut model = self.model.lock().unwrap();
    let query_vector = model.embed_query(query).ok()?.vector;
    let candidate_set: std::collections::HashSet<&str> = candidate_ids.iter().map(|id| id.as_str()).collect();
    let scored: Vec<(NoteId, f64)> = index.entries.iter()
        .filter(|e| candidate_set.contains(e.note_id.as_str()))
        .filter_map(|e| NoteId::parse(&e.note_id).ok().map(|id| (id, dense::cosine_similarity(&query_vector, &e.embedding) as f64)))
        .collect();
    Some(scored)
}
```

**Correction found while writing this section:** B4.2's own trait doc
comment says `score`'s `candidate_ids` restricts what the scorer may
return — re-reading it now, the real question is where
`candidate_ids` comes from in the first place. **B4.6 (search_impl
wiring, not this task) must decide: does `DenseV2` mode still call
`self.indexer.candidate_ids(query)` (the lexical candidate generator)
to get a candidate set, then ask the dense scorer to RANK that same
set? Or does `DenseV2` ask the dense scorer for its OWN candidates
(every note in the dense index, unfiltered by lexical overlap) and
skip lexical candidate generation entirely?** The evaluation harness's
own dense-standalone configuration (Priority 4 DEV/calibration/
held-out) did the latter — queried every note in the corpus by cosine
similarity, with no lexical pre-filtering at all, which is WHY it
could find notes the lexical path would miss entirely (the whole point
of dense retrieval). If B4.6 restricts dense scoring to only the
lexical candidate set, dense retrieval could never surface a note
lexical search didn't already find, defeating its own purpose and
contradicting every evaluation number in `frozen-config-v2.json`
(measured as dense-standalone, unfiltered). **This is flagged here,
explicitly, as a real architectural question for B4.6 to resolve
before wiring `search_impl`'s `DenseV2` branch — not resolved by this
task, since it is the search-dispatch question, not the scorer's own
job.** The `score` signature above is written to support EITHER
choice (it takes whatever `candidate_ids` it's given and restricts to
those), so this task's own implementation is not blocked by the
question — only B4.6's wiring is.

## 7. `ensure_loaded` — the fallback-trigger check

```rust
fn ensure_loaded<'a>(&self, guard: &'a MutexGuard<Option<DenseIndex>>) -> Option<&'a DenseIndex> {
    // (sketch -- real implementation loads-on-miss under the lock,
    // which needs a slightly different shape than a plain borrow;
    // exact signature is this task's own implementation detail, not
    // re-frozen here)
}
```

Checks, in order (B4.0's frozen fallback conditions, B4.3's design
doc Section 4): file missing → `None` (condition 1). Corrupted/
version-mismatched → `None` (condition 1, treated identically per
B4.3's own "any validation failure → same outcome" rule).
Model-identity-hash mismatch → `None` (condition 2). Otherwise →
`Some(&index)`.

## 8. STOP — design frozen, implementation follows
