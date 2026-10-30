//! B1 Task 3: `Bm25Index` — a standalone BM25 postings/scoring structure.
//!
//! `Bm25Index` has the same text-only contract `TrigramIndex` (Task 2) has:
//! `insert(note_id, text)` / `remove(note_id)` operate purely on `&str` /
//! `NoteId`, never on a `Document`, and never see structured metadata or
//! classification. It is handed only whatever `String` the caller computed
//! as `Document::text_searchable` — so the Sensitive-note exclusion property
//! this module's own tests check is the SAME property Task 2 already
//! verified for `TrigramIndex`, re-verified here because this is a different
//! surface (postings keyed on term frequency, not trigrams) that could leak
//! the same body text through an entirely different bug (A3's own lesson:
//! each surface needs its own dedicated test).
//!
//! # Tokenization
//!
//! Terms are whitespace-delimited, Unicode-aware (`char::is_whitespace`),
//! case-folded tokens (same case-insensitivity rationale as `TrigramIndex`:
//! nothing in the spec asks for case to be a distinguishing signal for
//! ranked search either). No punctuation stripping beyond that: a token is
//! exactly a maximal run of non-whitespace `char`s, lowercased. This keeps
//! tokenization trivially reproducible and matches `query_ranked`'s own
//! tokenization of the query string, so a query term always compares
//! equal-for-equal against an indexed term with the same surface form.
//!
//! # Scoring: hand-rolled BM25, no smoothing floor (brief Step 1b)
//!
//! This is the classic Robertson/Sparck-Jones BM25 formula:
//!
//! ```text
//! score(D, Q) = sum over each term t in Q of IDF(t) * ( f(t, D) * (k1 + 1) )
//!                                             -----------------------------
//!                                             f(t, D) + k1 * (1 - b + b * |D| / avgdl)
//! ```
//!
//! with `IDF(t) = ln( (N - n(t) + 0.5) / (n(t) + 0.5) + 1 )` (the standard
//! Lucene-style IDF that is always >= 0 — never negative — avoiding the
//! classic original-paper IDF's capacity to go negative for a term present
//! in more than half the corpus), `N` the number of indexed documents,
//! `n(t)` the number of indexed documents containing `t`, `f(t, D)` the raw
//! term frequency of `t` in document `D`, `|D|` the length (in tokens) of
//! `D`, and `avgdl` the average document length across the corpus. `k1` and
//! `b` are the usual tunable constants (1.2 and 0.75 here, the conventional
//! defaults).
//!
//! **This implementation has no smoothing/floor term.** A document whose
//! token set has zero overlap with the query contributes exactly zero to
//! every term in the sum (there is nothing to sum: a term absent from a
//! document is simply skipped for that document, per the standard formula,
//! rather than being given some minimum nonzero contribution). This is an
//! explicit implementation CHOICE being pinned here as a constraint, not an
//! assumed general property of "BM25": some BM25 variants (BM25+, certain
//! smoothing schemes used to avoid zero scores for term-frequency-saturated
//! long documents) DO guarantee a nonzero floor for every document
//! regardless of overlap. If `query_ranked` is ever asked to add such a
//! variant, Task 7's citation-correctness tests (which rely on "zero overlap
//! implies not cited, because its score is exactly zero and it is never a
//! top-k hit for that query") must be revisited at the same time.
//!
//! # Why `remove` exists (update support)
//!
//! Like `TrigramIndex::insert`, `Bm25Index::insert` first removes any prior
//! entry for the same note id before inserting the new one — the same
//! remove-then-reinsert pattern Task 2 established, needed because a write
//! to an existing note must replace its old postings and length
//! contribution to `avgdl`, not add a second, stale entry alongside the new
//! one.

use std::collections::HashMap;

use storage::NoteId;

/// BM25 tuning constants (conventional defaults; see the module doc comment
/// for the formula they appear in).
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// A BM25 postings/scoring index: per-note term-frequency maps, the
/// corpus-wide document-frequency count per term (how many documents
/// contain each term at least once, for IDF), and each note's own token
/// count (for `|D|` and `avgdl`).
#[derive(Debug, Default)]
pub struct Bm25Index {
    /// note id -> (term -> raw term frequency in that note's indexed text).
    term_freqs: HashMap<NoteId, HashMap<String, u32>>,
    /// note id -> total token count of that note's indexed text (`|D|`).
    doc_lengths: HashMap<NoteId, u32>,
    /// term -> number of distinct notes whose term-frequency map contains
    /// it at least once (`n(t)`), maintained incrementally by `insert` /
    /// `remove` rather than recomputed from `term_freqs` on every query.
    doc_freqs: HashMap<String, u32>,
}

impl Bm25Index {
    pub fn new() -> Self {
        Self { term_freqs: HashMap::new(), doc_lengths: HashMap::new(), doc_freqs: HashMap::new() }
    }

    /// Indexes `text` under `note_id`, replacing any previous entry for the
    /// same note id first (see the module doc comment's "why `remove`
    /// exists" section). A `text` that tokenizes to nothing (empty or
    /// all-whitespace) is still recorded as a zero-length document: it
    /// contributes no terms to any postings list, but it IS counted in `N`
    /// and in `avgdl`'s denominator, matching `TrigramIndex::insert`'s own
    /// "every inserted note id is a real entry" behavior and keeping
    /// `avgdl` an honest corpus-wide average rather than silently excluding
    /// empty documents from it.
    pub fn insert(&mut self, note_id: NoteId, text: &str) {
        self.remove(&note_id);

        let tokens = tokenize(text);
        let doc_length = tokens.len() as u32;

        let mut freqs: HashMap<String, u32> = HashMap::new();
        for token in tokens {
            *freqs.entry(token).or_insert(0) += 1;
        }

        for term in freqs.keys() {
            *self.doc_freqs.entry(term.clone()).or_insert(0) += 1;
        }

        self.doc_lengths.insert(note_id.clone(), doc_length);
        self.term_freqs.insert(note_id, freqs);
    }

    /// Removes every postings contribution previously added for `note_id`
    /// (its term-frequency map, its document-length contribution, and its
    /// contribution to every term's document frequency). Removing a note id
    /// that was never inserted is a no-op.
    pub fn remove(&mut self, note_id: &NoteId) {
        self.doc_lengths.remove(note_id);
        let Some(freqs) = self.term_freqs.remove(note_id) else {
            return;
        };
        for term in freqs.keys() {
            if let Some(count) = self.doc_freqs.get_mut(term) {
                *count -= 1;
                if *count == 0 {
                    self.doc_freqs.remove(term);
                }
            }
        }
    }

    /// Returns the top `k` note ids for `query`, ranked by BM25 score
    /// descending, each paired with its score. Only notes with a strictly
    /// positive score are returned (see the module doc comment: a note with
    /// zero query-term overlap scores exactly zero and is never included
    /// here, by construction — there is no floor to lift it above zero).
    /// Ties in score are broken by the note id's string form
    /// (lexicographic), purely for deterministic output ordering in tests
    /// (`NoteId` itself has no `Ord` impl); callers needing a different
    /// tie-break should not rely on this.
    pub fn query_ranked(&self, query: &str, k: usize) -> Vec<(NoteId, f64)> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() || self.term_freqs.is_empty() {
            return Vec::new();
        }

        let n = self.term_freqs.len() as f64;
        let avgdl = {
            let total: u64 = self.doc_lengths.values().map(|&len| len as u64).sum();
            total as f64 / self.doc_lengths.len().max(1) as f64
        };

        let mut scores: HashMap<NoteId, f64> = HashMap::new();
        for term in &query_terms {
            let Some(&doc_freq) = self.doc_freqs.get(term) else {
                // A query term absent from the WHOLE corpus contributes to
                // no document's score at all; skip it rather than computing
                // an IDF for a term that appears in zero documents.
                continue;
            };
            let idf = idf(n, doc_freq as f64);

            for (note_id, freqs) in &self.term_freqs {
                let Some(&tf) = freqs.get(term) else { continue };
                let doc_len = *self.doc_lengths.get(note_id).unwrap_or(&0) as f64;
                let tf = tf as f64;
                let denom = tf + K1 * (1.0 - B + B * doc_len / avgdl.max(1.0));
                let term_score = idf * (tf * (K1 + 1.0)) / denom;
                *scores.entry(note_id.clone()).or_insert(0.0) += term_score;
            }
        }

        let mut ranked: Vec<(NoteId, f64)> = scores.into_iter().filter(|(_, score)| *score > 0.0).collect();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.as_str().cmp(b.0.as_str()))
        });
        ranked.truncate(k);
        ranked
    }
}

/// The standard Lucene-style IDF: always >= 0, unlike the original
/// Robertson/Sparck-Jones formula's `ln((N - n + 0.5) / (n + 0.5))`, which
/// can go negative for a term present in more than half the corpus. See the
/// module doc comment.
fn idf(n: f64, doc_freq: f64) -> f64 {
    ((n - doc_freq + 0.5) / (doc_freq + 0.5) + 1.0).ln()
}

/// Case-folded, whitespace-delimited tokenization (Unicode-aware). See the
/// module doc comment.
fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace().map(|token| token.to_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> NoteId {
        NoteId::parse(s).unwrap()
    }

    // --- Step 1: a note containing ALL query terms ranks above a note
    // containing only SOME of them. ---

    #[test]
    fn a_note_with_all_query_terms_ranks_above_a_note_with_only_some() {
        let mut index = Bm25Index::new();
        index.insert(id("note-all"), "rust programming language systems");
        index.insert(id("note-some"), "rust is fun");
        // A filler note so the corpus has more than two documents, keeping
        // IDF/avgdl computations realistic rather than a degenerate N=2 case.
        index.insert(id("note-filler"), "completely unrelated content about gardening");

        let hits = index.query_ranked("rust programming language", 10);

        let positions: HashMap<&str, usize> =
            hits.iter().enumerate().map(|(i, (id, _))| (id.as_str(), i)).collect();

        assert!(positions.contains_key("note-all"), "expected note-all to be a hit: {hits:?}");
        assert!(positions.contains_key("note-some"), "expected note-some to be a hit: {hits:?}");
        assert!(
            positions["note-all"] < positions["note-some"],
            "note matching all query terms must rank above a note matching only some: {hits:?}"
        );
    }

    // --- Step 1b: a note with ZERO overlapping terms scores exactly zero,
    // not a nonzero floor (explicit implementation constraint, not an
    // assumed general BM25 property). ---

    #[test]
    fn a_note_with_zero_overlapping_terms_is_never_a_hit_no_nonzero_floor() {
        let mut index = Bm25Index::new();
        index.insert(id("note-match"), "rust programming language");
        index.insert(id("note-no-overlap"), "gardening tomatoes zucchini");

        let hits = index.query_ranked("rust programming", 10);

        let hit_ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(hit_ids.contains(&"note-match"));
        assert!(
            !hit_ids.contains(&"note-no-overlap"),
            "a note with zero query-term overlap must score exactly zero and never appear in ranked results: {hits:?}"
        );
    }

    #[test]
    fn query_ranked_returns_no_results_for_an_empty_corpus() {
        let index = Bm25Index::new();
        assert!(index.query_ranked("anything", 10).is_empty());
    }

    #[test]
    fn query_ranked_respects_the_k_limit() {
        let mut index = Bm25Index::new();
        for i in 0..5 {
            index.insert(id(&format!("note-{i}")), "shared term appears in every note");
        }

        let hits = index.query_ranked("shared term", 3);

        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn remove_then_requery_returns_no_hits_for_that_note_id() {
        let mut index = Bm25Index::new();
        index.insert(id("note-a"), "a distinctive_term appears here");
        index.insert(id("note-b"), "unrelated filler content entirely");

        assert!(index.query_ranked("distinctive_term", 10).iter().any(|(id, _)| id.as_str() == "note-a"));

        index.remove(&id("note-a"));

        assert!(index.query_ranked("distinctive_term", 10).is_empty());
    }

    #[test]
    fn reinserting_a_note_id_replaces_its_previous_postings() {
        let mut index = Bm25Index::new();
        index.insert(id("note-a"), "old_term_value here");
        index.insert(id("note-b"), "filler content to avoid a degenerate single-doc corpus");
        assert!(index.query_ranked("old_term_value", 10).iter().any(|(id, _)| id.as_str() == "note-a"));

        index.insert(id("note-a"), "new_term_value here");

        assert!(index.query_ranked("old_term_value", 10).is_empty());
        assert!(index.query_ranked("new_term_value", 10).iter().any(|(id, _)| id.as_str() == "note-a"));
    }

    #[test]
    fn case_insensitive_matching() {
        let mut index = Bm25Index::new();
        index.insert(id("note-a"), "a function named Get_User_By_Id lives here");
        index.insert(id("note-b"), "unrelated filler content entirely");

        let hits = index.query_ranked("GET_USER_BY_ID", 10);

        assert!(hits.iter().any(|(id, _)| id.as_str() == "note-a"));
    }

    // --- Step 3: a Sensitive note's body text is never inserted into the
    // BM25 index (same adversarial pattern as Task 2's TrigramIndex test,
    // for this second index structure). ---

    #[test]
    fn a_distinctive_marker_never_passed_to_insert_is_never_findable() {
        let mut index = Bm25Index::new();
        // Simulates what a Sensitive note's `text_searchable` looks like:
        // only structured fields, built by `document::build_text_searchable`
        // — the body's distinctive marker is simply never part of `text`.
        let sensitive_text_searchable = "note-id-only bug draft internal sensitive project-x 2026-10-30 2026-10-30 2026-10-30 1";
        index.insert(id("sensitive-note"), sensitive_text_searchable);
        index.insert(id("note-filler"), "some other note with different content entirely");

        let hits = index.query_ranked("SECRET-MARKER-BODY-CONTENT", 10);

        assert!(hits.is_empty(), "a marker string never inserted must never be found");
    }
}
