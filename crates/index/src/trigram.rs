//! B1 Task 2: `TrigramIndex` — a standalone trigram postings structure.
//!
//! `TrigramIndex` operates purely on text: `insert(note_id, text)` /
//! `remove(note_id)` / `query_exact(term)`. It never sees a `Document`, never
//! sees structured metadata, and never sees classification — it is handed
//! only whatever `String` the caller computed as `Document::text_searchable`
//! (per the Task 2 brief's final paragraph: the postings structures consume
//! ONLY text-searchable content, never structured-metadata-only fields).
//! That separation is what makes the `Sensitive`-note exclusion tests in
//! this file meaningful: if `TrigramIndex` is never given the body/`source`/
//! `relations` text for a `Sensitive` note in the first place (because
//! `Document::text_searchable` already excluded them), there is nothing to
//! accidentally leak into its postings.
//!
//! # Tokenization design (brief Step 5 — decided and documented here)
//!
//! **Case folding: trigram matching is case-INSENSITIVE.** Both `insert`
//! and `query_exact` lowercase their input before extracting trigrams. The
//! spec's own worked example is a lowercase identifier
//! (`get_user_by_id`), and folding case means a query for `get_user_by_id`
//! still matches if the identifier ever appears differently-cased in a
//! note body (e.g. inside a code block someone capitalized, or a markdown
//! heading) — case-insensitive is the more forgiving, more generally
//! useful default for a search feature, and nothing in the spec asks for
//! case to be a distinguishing signal. The cost is the usual one for
//! case-folded search (a query can no longer force a case-sensitive exact
//! match), which is accepted here as the right trade-off for this feature.
//!
//! **Unicode: trigrams are 3-`char` (Unicode scalar value) windows, not
//! 3-byte windows.** Note bodies are explicitly mixed English/Italian
//! Markdown per the project's own content, so multi-byte UTF-8 (e.g. `à`,
//! `è`, `ò`) is expected, not an edge case. Slicing raw bytes would split
//! multi-byte characters mid-sequence and produce garbage trigrams (or
//! panic on a non-UTF-8-boundary slice); iterating `char`s instead means a
//! 3-character window is always three whole Unicode scalar values,
//! regardless of how many bytes each occupies. Case-folding is also done
//! on `char`s (`char::to_lowercase`, which exists for non-ASCII scalars
//! too, e.g. `À` -> `à`) before the trigram windows are taken, so folding
//! and Unicode-correctness compose rather than fighting each other.
//!
//! A term shorter than 3 characters has no trigrams at all under this
//! scheme and therefore cannot be found by `query_exact` (consistent with
//! trigram indexing generally — this is a known, accepted limitation, not
//! a bug: Task 3's `Bm25Index` is the structure responsible for whole-term
//! / short-term matching, not `TrigramIndex`).

use std::collections::{HashMap, HashSet};

use storage::NoteId;

/// A trigram postings index: trigram -> set of note ids whose indexed text
/// contains that trigram, the reverse mapping (note id -> trigrams it
/// contributed) so `remove` can clean up exactly what `insert` added, and
/// the original (case-folded) indexed text per note id so `query_exact` can
/// confirm a real substring match on top of the trigram pre-filter.
#[derive(Debug, Default)]
pub struct TrigramIndex {
    postings: HashMap<[char; 3], HashSet<NoteId>>,
    by_note: HashMap<NoteId, HashSet<[char; 3]>>,
    /// Case-folded indexed text per note id, used only by `query_exact` to
    /// verify a candidate actually contains the query term as a literal
    /// substring (see `contains_term`). Kept in sync with `by_note`: every
    /// `insert` populates both, every `remove` clears both.
    texts: HashMap<NoteId, String>,
}

impl TrigramIndex {
    pub fn new() -> Self {
        Self { postings: HashMap::new(), by_note: HashMap::new(), texts: HashMap::new() }
    }

    /// Indexes `text` under `note_id`. If `note_id` was already indexed,
    /// its previous trigrams are removed first (so `insert` is also the
    /// correct way to re-index a note whose content changed, without a
    /// separate `update` method: callers needing "idempotent re-insert"
    /// just call `insert` again with the new text).
    pub fn insert(&mut self, note_id: NoteId, text: &str) {
        self.remove(&note_id);

        let trigrams = trigrams_of(text);
        if trigrams.is_empty() {
            return;
        }

        for trigram in &trigrams {
            self.postings.entry(*trigram).or_default().insert(note_id.clone());
        }
        self.texts.insert(note_id.clone(), text.to_lowercase());
        self.by_note.insert(note_id, trigrams);
    }

    /// Removes every trigram posting previously added for `note_id`, along
    /// with its stored text. A subsequent `query_exact` for any of its terms
    /// returns no hit for this note id. Removing a note id that was never
    /// inserted is a no-op.
    pub fn remove(&mut self, note_id: &NoteId) {
        self.texts.remove(note_id);
        let Some(trigrams) = self.by_note.remove(note_id) else {
            return;
        };
        for trigram in trigrams {
            if let Some(notes) = self.postings.get_mut(&trigram) {
                notes.remove(note_id);
                if notes.is_empty() {
                    self.postings.remove(&trigram);
                }
            }
        }
    }

    /// Returns every note id whose indexed text contains `term` as an exact
    /// (case-insensitive) substring. Implementation: intersects the postings
    /// lists of all of `term`'s trigrams to get a cheap CANDIDATE set (every
    /// one of `term`'s trigrams occurs somewhere in the note's text, but not
    /// necessarily contiguously or in `term`'s exact order), then confirms
    /// each candidate with `contains_term`, which checks the literal
    /// substring against the note's stored text. The trigram intersection is
    /// a fast pre-filter; `contains_term` is what makes the result actually
    /// correct as an exact-match primitive.
    ///
    /// A `term` shorter than 3 characters has no trigrams and always
    /// returns an empty result (see the module doc comment).
    pub fn query_exact(&self, term: &str) -> Vec<NoteId> {
        let term_trigrams = trigrams_of(term);
        if term_trigrams.is_empty() {
            return Vec::new();
        }

        let mut candidates: Option<HashSet<NoteId>> = None;
        for trigram in &term_trigrams {
            let Some(notes) = self.postings.get(trigram) else {
                // A required trigram has no postings at all: no note can
                // possibly match.
                return Vec::new();
            };
            candidates = Some(match candidates {
                None => notes.clone(),
                Some(acc) => acc.intersection(notes).cloned().collect(),
            });
            if candidates.as_ref().is_some_and(HashSet::is_empty) {
                return Vec::new();
            }
        }

        candidates
            .unwrap_or_default()
            .into_iter()
            .filter(|note_id| self.contains_term(note_id, term))
            .collect()
    }

    /// Confirms that `note_id`'s indexed text actually contains `term` as a
    /// literal (case-insensitive) substring. This is the verification step
    /// `query_exact` runs over each trigram-intersection candidate, since
    /// the intersection alone only proves all of `term`'s trigrams are
    /// present somewhere in the text, not that they occur contiguously in
    /// `term`'s exact order.
    fn contains_term(&self, note_id: &NoteId, term: &str) -> bool {
        self.texts.get(note_id).is_some_and(|text| text.contains(&term.to_lowercase()))
    }
}

/// Case-folds `text` (per-`char`, Unicode-aware) and returns its set of
/// distinct 3-`char` trigrams. Returns an empty set for text with fewer
/// than 3 characters.
fn trigrams_of(text: &str) -> HashSet<[char; 3]> {
    let folded: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();

    if folded.len() < 3 {
        return HashSet::new();
    }

    folded.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> NoteId {
        NoteId::parse(s).unwrap()
    }

    // --- Step 1: exact identifier, appears once, found. ---

    #[test]
    fn identifier_appearing_exactly_once_in_one_note_body_is_found_by_exact_query() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "some text mentioning get_user_by_id exactly once here");
        index.insert(id("note-b"), "unrelated content about something else entirely");

        let hits = index.query_exact("get_user_by_id");

        assert_eq!(hits, vec![id("note-a")]);
    }

    // --- Step 2: Sensitive note's body text never reaches the index. ---
    // TrigramIndex itself has no notion of classification; this proves
    // that if a caller never hands it the body text in the first place
    // (the Document/allowlist boundary), the marker is genuinely absent
    // from postings, not just "not findable by coincidence."

    #[test]
    fn a_distinctive_marker_never_passed_to_insert_is_never_findable() {
        let mut index = TrigramIndex::new();
        // Simulates what a Sensitive note's `text_searchable` looks like:
        // only structured fields, built by `document::build_text_searchable`
        // — the body's distinctive marker is simply never part of `text`.
        let sensitive_text_searchable = "note-id-only bug draft internal sensitive project-x 2026-10-30 2026-10-30 2026-10-30 1";
        index.insert(id("sensitive-note"), sensitive_text_searchable);

        let hits = index.query_exact("SECRET-MARKER-BODY-CONTENT");

        assert!(hits.is_empty(), "a marker string never inserted must never be found");
    }

    // --- Step 2b: Sensitive note's `source` field never reaches the index. ---

    #[test]
    fn a_distinctive_source_marker_never_passed_to_insert_is_never_findable() {
        let mut index = TrigramIndex::new();
        // `source` is deliberately excluded from a Sensitive note's
        // text_searchable even though it's a structured field (see
        // document.rs's module doc comment) — simulate that by never
        // including it in what gets inserted.
        let sensitive_text_searchable = "note-id-only bug draft internal sensitive project-x 2026-10-30 2026-10-30 2026-10-30 1";
        index.insert(id("sensitive-note"), sensitive_text_searchable);

        let hits = index.query_exact("agent:coder@prompt-v3-MARKER");

        assert!(hits.is_empty(), "a source marker string never inserted must never be found");
    }

    // --- Step 3: remove then re-query returns no hits. ---

    #[test]
    fn remove_then_requery_returns_no_hits_for_that_note_id() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "contains the term uniqueterm123 here");
        assert_eq!(index.query_exact("uniqueterm123"), vec![id("note-a")]);

        index.remove(&id("note-a"));

        assert!(index.query_exact("uniqueterm123").is_empty());
    }

    #[test]
    fn remove_of_one_note_does_not_affect_another_notes_shared_term() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "shared_identifier appears here");
        index.insert(id("note-b"), "shared_identifier appears here too");

        index.remove(&id("note-a"));

        let hits = index.query_exact("shared_identifier");
        assert_eq!(hits, vec![id("note-b")]);
    }

    #[test]
    fn remove_of_unknown_note_id_is_a_no_op() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "some content");

        index.remove(&id("note-never-inserted"));

        assert_eq!(index.query_exact("content"), vec![id("note-a")]);
    }

    // --- Step 5: case folding. ---

    #[test]
    fn query_is_case_insensitive() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "a function named Get_User_By_Id lives here");

        assert_eq!(index.query_exact("get_user_by_id"), vec![id("note-a")]);
        assert_eq!(index.query_exact("GET_USER_BY_ID"), vec![id("note-a")]);
    }

    #[test]
    fn insert_is_case_insensitive_at_insertion_time_too() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "UPPERCASE TERM here");

        // A lowercase query finds text that was originally uppercase.
        assert_eq!(index.query_exact("uppercase"), vec![id("note-a")]);
    }

    // --- Step 5: Unicode round-trip (non-ASCII, Italian). ---

    #[test]
    fn non_ascii_italian_term_round_trips_through_tokenization_and_query() {
        let mut index = TrigramIndex::new();
        // "città" (city) and "perché" (because/why) — accented multi-byte
        // UTF-8 characters, mixed with ASCII, as real note bodies are.
        index.insert(id("note-a"), "la città è grande, perché cresce ogni anno");

        assert_eq!(index.query_exact("città"), vec![id("note-a")]);
        assert_eq!(index.query_exact("perché"), vec![id("note-a")]);
        // Case-insensitivity also holds for accented characters.
        assert_eq!(index.query_exact("CITTÀ"), vec![id("note-a")]);
    }

    #[test]
    fn term_shorter_than_three_chars_has_no_trigrams_and_finds_nothing() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "ab cd ef");

        assert!(index.query_exact("ab").is_empty());
    }

    #[test]
    fn query_requiring_a_trigram_not_present_anywhere_returns_no_hits() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "completely unrelated content");

        assert!(index.query_exact("zzzzz-not-present").is_empty());
    }

    // --- Reviewer finding: trigram-intersection alone produces false
    // positives. All four of "catdog"'s trigrams (cat, atd, tdo, dog) can
    // each appear somewhere in a text that never contains "catdog" as a
    // literal substring. query_exact must verify the actual substring, not
    // just that all its trigrams are individually present.

    #[test]
    fn query_exact_rejects_a_trigram_intersection_false_positive() {
        let mut index = TrigramIndex::new();
        // "cat" (cat is nice), "atd" (nice, atd), "tdo" (atd is a typo, tdo),
        // "dog" (the dog barks) each occur somewhere in this text, but the
        // literal substring "catdog" never occurs anywhere in it.
        index.insert(id("note-a"), "cat is nice, atd is a typo, tdo what, the dog barks");

        let hits = index.query_exact("catdog");

        assert!(hits.is_empty(), "catdog never occurs literally, so it must not be a hit");
    }

    #[test]
    fn reinserting_a_note_id_replaces_its_previous_postings() {
        let mut index = TrigramIndex::new();
        index.insert(id("note-a"), "old_term_value here");
        assert_eq!(index.query_exact("old_term_value"), vec![id("note-a")]);

        index.insert(id("note-a"), "new_term_value here");

        assert!(index.query_exact("old_term_value").is_empty());
        assert_eq!(index.query_exact("new_term_value"), vec![id("note-a")]);
    }
}
