//! B5 remediation M5, Task 1: pure, filesystem-independent hash-chain
//! and recovery-scan primitives for a future durable audit log.
//!
//! **Scope discipline (Task 1's own original authorization):** this
//! module contains ONLY pure functions over in-memory byte slices — no
//! file I/O, no locking, no DPAPI. `AuditEventKind`/production wiring
//! stayed out of THIS module even after Task 4 gave it real callers —
//! see `audit_record.rs` (the concrete line format) and
//! `brain-api-audit-worker` (the actual filesystem-writing worker,
//! M5 Task 4) for where that lives instead.
//!
//! **Task 4 update**: a narrow set of this module's items are now
//! re-exported from this crate's own `lib.rs` (see that file's own `pub
//! use audit_chain::{...}` doc comment for exactly which ones and why)
//! so `brain-api-audit-worker` can reuse this module's pure chain
//! verification rather than reimplementing it. The module itself stays
//! private (`mod audit_chain;`, not `pub mod`) — only the facade's
//! chosen items are reachable from outside this crate.
//!
//! **Design basis**, reusing an established pattern already proven in
//! this codebase (`crates/vault-security/src/deletion_log.rs`'s own
//! `chain_hash = sha256(prev_chain_hash || generation || note_id)`
//! formula and genesis convention), not inventing a new one:
//! `record_hash = sha256(prev_record_hash || sequence || content_bytes)`,
//! with genesis `prev_record_hash` = 64 ASCII `'0'` characters — the
//! exact same genesis convention `deletion_log.rs` already uses, chosen
//! for consistency within this codebase rather than arbitrarily.
//!
//! **What this module's chain verification proves, precisely, and what
//! it does not** (the distinction the M5 design review required): a
//! verified chain from genesis through record N proves every byte of
//! records 0..=N is exactly as it was when record N's hash was computed
//! — it proves INTERNAL CONSISTENCY, never AUTHENTICITY (anyone who can
//! write the file can compute these same hashes) and never FRESHNESS
//! (a wholesale replay of an old-but-internally-consistent prefix is
//! indistinguishable from the real history by this module alone — see
//! continuity decision `d279ua`). Detecting authenticity/freshness is
//! the job of a future, separate anchor mechanism (DPAPI-wrapped head
//! reference on Windows, explicitly NOT built in this task), not this
//! module.

use sha2::{Digest, Sha256};

/// Genesis `prev_record_hash`: 64 ASCII `'0'` characters, the exact same
/// convention `deletion_log.rs`'s own genesis reference already uses in
/// this codebase (`vault-security::deletion_log`'s
/// `is_genesis_reference`/chain-hash doc comments) — reused for
/// consistency, not reinvented.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One parsed, structurally valid record as read from the (future) log
/// file — this type carries no filesystem concept of its own; a caller
/// supplies `content_bytes` however it obtains them (file I/O is a later
/// task's job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRecord {
    pub sequence: u64,
    pub prev_record_hash: String,
    pub record_hash: String,
    /// The exact bytes the hash was computed over — opaque to this
    /// module (the future event schema's own serialized form; this
    /// module never inspects or interprets this content).
    pub content_bytes: Vec<u8>,
}

/// Computes `sha256(prev_record_hash || sequence || content_bytes)` as a
/// lowercase hex string. `sequence` is encoded as its decimal ASCII
/// representation (matching `deletion_log.rs`'s own
/// `format!("{generation}:{chain_hash}")`-style plain-text concatenation
/// convention, not a fixed-width binary encoding) — chosen so the
/// formula is trivially reproducible by a human auditor reading the log
/// by hand, not just by this module's own code.
pub fn compute_record_hash(prev_record_hash: &str, sequence: u64, content_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_record_hash.as_bytes());
    hasher.update(sequence.to_string().as_bytes());
    hasher.update(content_bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Verifies that `record.record_hash` is exactly what
/// [`compute_record_hash`] would produce for `record`'s own
/// `prev_record_hash`/`sequence`/`content_bytes` — a pure equality
/// check, no I/O, no mutation.
pub fn verify_record_hash(record: &ChainRecord) -> bool {
    compute_record_hash(
        &record.prev_record_hash,
        record.sequence,
        &record.content_bytes,
    ) == record.record_hash
}

/// Why one record failed to extend the chain from its predecessor —
/// distinguishing the specific failure precisely, since the M5 design
/// review requires this module to distinguish "a verifiable prefix" from
/// "permission to resume writing" rather than collapsing every failure
/// into one generic "invalid" bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainLinkError {
    /// `record.prev_record_hash` does not equal the previous record's
    /// own (already-verified) `record_hash` — a broken link, whether
    /// from corruption or a genuine fork.
    BrokenPrevHash,
    /// `record.record_hash` does not equal
    /// `compute_record_hash(record.prev_record_hash, record.sequence,
    /// record.content_bytes)` — the record's own content/hash pairing is
    /// internally inconsistent (tampered content, or a corrupted hash
    /// field).
    BrokenRecordHash,
    /// `record.sequence` is not exactly `expected_sequence` (the
    /// immediately-preceding verified record's sequence + 1, or 0 for
    /// the very first record) — a gap or a duplicate.
    UnexpectedSequence { expected: u64, found: u64 },
}

/// Verifies that `record` correctly extends a chain whose last verified
/// state is `(expected_sequence, prev_record_hash)` — the one function
/// every higher-level scan below is built from. Pure: takes exactly the
/// state needed, mutates nothing, performs no I/O.
pub fn verify_link(
    prev_record_hash: &str,
    expected_sequence: u64,
    record: &ChainRecord,
) -> Result<(), ChainLinkError> {
    if record.sequence != expected_sequence {
        return Err(ChainLinkError::UnexpectedSequence {
            expected: expected_sequence,
            found: record.sequence,
        });
    }
    if record.prev_record_hash != prev_record_hash {
        return Err(ChainLinkError::BrokenPrevHash);
    }
    if !verify_record_hash(record) {
        return Err(ChainLinkError::BrokenRecordHash);
    }
    Ok(())
}

/// One line's own parse/structural outcome, independent of chain
/// linkage — a line can fail to even become a [`ChainRecord`] before
/// chain verification is ever attempted (e.g. malformed JSON in the real
/// future file format; this module stays serialization-format-agnostic,
/// so a caller supplies this classification itself rather than this
/// module parsing any concrete wire format).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineOutcome {
    /// The line parsed into a well-formed [`ChainRecord`] — chain
    /// linkage is checked separately by the scan below.
    Parsed(ChainRecord),
    /// The line could not be parsed into a [`ChainRecord`] at all (the
    /// caller's own parse step failed) — classified `is_physical_tail`
    /// by the caller (the one fact this module cannot determine itself,
    /// since it has no concept of "end of file" — that is inherently a
    /// filesystem fact a future I/O-aware caller supplies).
    Unparseable { is_physical_tail: bool },
}

/// The result of scanning a sequence of [`LineOutcome`]s from genesis —
/// the exact "verifiable prefix vs. permission to resume" distinction
/// the M5 design review required, made explicit as separate fields
/// rather than a single verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// Every record from the start up to and including the last one
    /// that verified cleanly — this is the ENTIRE verifiable prefix,
    /// regardless of what follows it.
    pub verified_records: Vec<ChainRecord>,
    /// The chain state (`sequence`, `record_hash`) the next NEW record
    /// would need to extend, if one were ever appended — `None` only
    /// when `verified_records` is empty (genesis: a first record would
    /// use `GENESIS_HASH`/`sequence: 0`).
    pub last_verified: Option<(u64, String)>,
    /// What stopped the scan from verifying anything further, if
    /// anything did. `None` means every supplied line verified cleanly
    /// end to end (the normal case).
    pub stop_reason: Option<ScanStopReason>,
}

/// Precisely which situation stopped the scan — this is what a future,
/// I/O-aware caller uses to decide whether resuming append is safe, not
/// this module itself (this module never decides "may I write"; it only
/// classifies what it found).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanStopReason {
    /// The very next line after the verified prefix was unparseable AND
    /// sits at the true physical end of the input (the caller marked
    /// it `is_physical_tail: true`) — the one case a future recovery
    /// procedure may act on (M5's later, separate "partial trailing
    /// record" task — never auto-resumed by this module).
    PartialTrailingRecord,
    /// An unparseable line was found that is NOT at the physical tail
    /// (more lines follow it) — corruption in the middle of the file,
    /// never treated as "just a crash."
    UnparseableNotAtTail,
    /// A line parsed structurally but failed chain linkage (see
    /// [`ChainLinkError`]) — corruption, a sequence gap/duplicate, or a
    /// genuine fork, at the given 0-based position in the supplied
    /// input sequence.
    LinkBroken {
        at_input_index: usize,
        error: ChainLinkError,
    },
}

/// Scans `lines` from genesis, verifying each one extends the chain
/// correctly, and classifies exactly where and why verification stopped
/// (if it did) — a pure function over an in-memory sequence, with no
/// concept of files, offsets, or byte lengths (a future I/O-aware caller
/// maps `at_input_index`/`is_physical_tail` back onto real file
/// positions itself).
///
/// Verifies the ENTIRE supplied sequence even after finding a problem
/// only up to the point the classification above requires (one stop
/// reason, not a list) — matching the design review's own requirement
/// that corruption NOT at the true tail must be distinguished from a
/// true partial-tail case, which requires looking at what (if anything)
/// follows the first problem, not stopping blindly at the first line
/// that fails.
pub fn scan_chain(lines: &[LineOutcome]) -> ScanResult {
    let mut verified_records = Vec::new();
    let mut expected_sequence: u64 = 0;
    let mut prev_hash = GENESIS_HASH.to_string();

    for (index, line) in lines.iter().enumerate() {
        match line {
            LineOutcome::Parsed(record) => match verify_link(&prev_hash, expected_sequence, record)
            {
                Ok(()) => {
                    prev_hash = record.record_hash.clone();
                    expected_sequence += 1;
                    verified_records.push(record.clone());
                }
                Err(error) => {
                    let last_verified = if verified_records.is_empty() {
                        None
                    } else {
                        Some((expected_sequence - 1, prev_hash.clone()))
                    };
                    return ScanResult {
                        verified_records,
                        last_verified,
                        stop_reason: Some(ScanStopReason::LinkBroken {
                            at_input_index: index,
                            error,
                        }),
                    };
                }
            },
            LineOutcome::Unparseable { is_physical_tail } => {
                let last_verified = if verified_records.is_empty() {
                    None
                } else {
                    Some((expected_sequence - 1, prev_hash.clone()))
                };
                let is_true_tail = *is_physical_tail && index == lines.len() - 1;
                let stop_reason = if is_true_tail {
                    ScanStopReason::PartialTrailingRecord
                } else {
                    ScanStopReason::UnparseableNotAtTail
                };
                return ScanResult {
                    verified_records,
                    last_verified,
                    stop_reason: Some(stop_reason),
                };
            }
        }
    }

    let last_verified = if verified_records.is_empty() {
        None
    } else {
        Some((expected_sequence - 1, prev_hash))
    };
    ScanResult {
        verified_records,
        last_verified,
        stop_reason: None,
    }
}

/// Whether [`scan_chain`]'s result permits a future caller to resume
/// appending NEW records immediately, without any recovery procedure —
/// this module's own, explicit, narrow answer to "permission to resume
/// writing," kept separate from "what is the verifiable prefix"
/// ([`ScanResult::verified_records`] answers that regardless of this
/// function's answer).
///
/// Only `None` (nothing stopped the scan — every supplied line verified)
/// permits immediate resumption. [`ScanStopReason::PartialTrailingRecord`]
/// requires a SEPARATE, explicit recovery procedure (a later M5 task,
/// not implemented here) before resuming — this function says so by
/// returning `false`, never `true`, for that case; this module does not
/// implement that recovery procedure itself, only names the fact that
/// one is required. Every other stop reason requires the SAME `false`
/// answer, with no recovery procedure proposed for them at all (a human
/// must resolve them).
pub fn may_resume_without_recovery(result: &ScanResult) -> bool {
    result.stop_reason.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(sequence: u64, prev_record_hash: &str, content: &[u8]) -> ChainRecord {
        let record_hash = compute_record_hash(prev_record_hash, sequence, content);
        ChainRecord {
            sequence,
            prev_record_hash: prev_record_hash.to_string(),
            record_hash,
            content_bytes: content.to_vec(),
        }
    }

    fn valid_chain(n: usize) -> Vec<ChainRecord> {
        let mut records = Vec::with_capacity(n);
        let mut prev = GENESIS_HASH.to_string();
        for i in 0..n {
            let r = record(i as u64, &prev, format!("event-{i}").as_bytes());
            prev = r.record_hash.clone();
            records.push(r);
        }
        records
    }

    // --- compute_record_hash / verify_record_hash ---

    #[test]
    fn compute_record_hash_is_deterministic_for_identical_inputs() {
        let a = compute_record_hash(GENESIS_HASH, 0, b"hello");
        let b = compute_record_hash(GENESIS_HASH, 0, b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn compute_record_hash_differs_when_prev_hash_sequence_or_content_differs() {
        let base = compute_record_hash(GENESIS_HASH, 0, b"hello");
        assert_ne!(
            base,
            compute_record_hash("deadbeef", 0, b"hello"),
            "different prev_hash must change the result"
        );
        assert_ne!(
            base,
            compute_record_hash(GENESIS_HASH, 1, b"hello"),
            "different sequence must change the result"
        );
        assert_ne!(
            base,
            compute_record_hash(GENESIS_HASH, 0, b"world"),
            "different content must change the result"
        );
    }

    #[test]
    fn compute_record_hash_is_a_64_char_lowercase_hex_string() {
        let hash = compute_record_hash(GENESIS_HASH, 0, b"hello");
        assert_eq!(hash.len(), 64);
        assert!(hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn genesis_hash_is_64_ascii_zero_characters() {
        assert_eq!(GENESIS_HASH.len(), 64);
        assert!(GENESIS_HASH.chars().all(|c| c == '0'));
    }

    #[test]
    fn verify_record_hash_accepts_a_correctly_computed_record_and_rejects_a_tampered_one() {
        let r = record(0, GENESIS_HASH, b"hello");
        assert!(verify_record_hash(&r));
        let mut tampered = r.clone();
        tampered.content_bytes = b"goodbye".to_vec();
        assert!(
            !verify_record_hash(&tampered),
            "a record whose content no longer matches its own stored hash must be rejected"
        );
    }

    // --- verify_link ---

    #[test]
    fn verify_link_accepts_a_correctly_chained_record() {
        let r = record(0, GENESIS_HASH, b"hello");
        assert_eq!(verify_link(GENESIS_HASH, 0, &r), Ok(()));
    }

    #[test]
    fn verify_link_rejects_a_sequence_gap() {
        let r = record(5, GENESIS_HASH, b"hello");
        assert_eq!(
            verify_link(GENESIS_HASH, 0, &r),
            Err(ChainLinkError::UnexpectedSequence {
                expected: 0,
                found: 5
            })
        );
    }

    #[test]
    fn verify_link_rejects_a_duplicate_sequence() {
        let r = record(0, GENESIS_HASH, b"hello");
        assert_eq!(
            verify_link(GENESIS_HASH, 1, &r),
            Err(ChainLinkError::UnexpectedSequence {
                expected: 1,
                found: 0
            })
        );
    }

    #[test]
    fn verify_link_rejects_a_prev_hash_mismatch() {
        let r = record(0, "wrong-prev-hash", b"hello");
        assert_eq!(
            verify_link(GENESIS_HASH, 0, &r),
            Err(ChainLinkError::BrokenPrevHash)
        );
    }

    #[test]
    fn verify_link_rejects_a_record_hash_mismatch_from_tampered_content() {
        let mut r = record(0, GENESIS_HASH, b"hello");
        r.content_bytes = b"tampered".to_vec();
        assert_eq!(
            verify_link(GENESIS_HASH, 0, &r),
            Err(ChainLinkError::BrokenRecordHash)
        );
    }

    // --- scan_chain: valid prefixes ---

    #[test]
    fn scan_chain_on_an_empty_input_verifies_nothing_and_permits_resumption() {
        let result = scan_chain(&[]);
        assert_eq!(result.verified_records, vec![]);
        assert_eq!(result.last_verified, None);
        assert_eq!(result.stop_reason, None);
        assert!(may_resume_without_recovery(&result));
    }

    #[test]
    fn scan_chain_on_a_fully_valid_chain_verifies_every_record_and_permits_resumption() {
        let records = valid_chain(10);
        let lines: Vec<LineOutcome> = records.iter().cloned().map(LineOutcome::Parsed).collect();
        let result = scan_chain(&lines);
        assert_eq!(result.verified_records, records);
        assert_eq!(result.stop_reason, None);
        assert!(may_resume_without_recovery(&result));
        let (seq, hash) = result.last_verified.unwrap();
        assert_eq!(seq, 9);
        assert_eq!(hash, records[9].record_hash);
    }

    // --- scan_chain: corruption (link broken mid-chain or at tail, not a parse failure) ---

    #[test]
    fn scan_chain_detects_a_single_byte_content_tamper_in_the_middle_and_trusts_the_verified_prefix(
    ) {
        let mut records = valid_chain(10);
        records[4].content_bytes = b"TAMPERED".to_vec(); // breaks record 4's own hash
        let lines: Vec<LineOutcome> = records.iter().cloned().map(LineOutcome::Parsed).collect();
        let result = scan_chain(&lines);
        assert_eq!(
            result.verified_records.len(),
            4,
            "records 0..=3 must still be trusted"
        );
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::LinkBroken {
                at_input_index: 4,
                error: ChainLinkError::BrokenRecordHash
            })
        );
        assert!(
            !may_resume_without_recovery(&result),
            "a broken link anywhere must never permit silent resumption"
        );
    }

    #[test]
    fn scan_chain_detects_a_broken_link_even_when_it_is_the_last_record_not_merely_unparseable() {
        // A STRUCTURALLY VALID but WRONGLY-CHAINED last record (e.g. a
        // forked/duplicated record) must be classified as LinkBroken, not
        // confused with a genuine PartialTrailingRecord (which requires an
        // UNPARSEABLE line, not a well-formed-but-wrong one).
        let mut records = valid_chain(5);
        records[4] = record(4, "0".repeat(64).as_str(), b"forked"); // wrong prev_hash
        let lines: Vec<LineOutcome> = records.iter().cloned().map(LineOutcome::Parsed).collect();
        let result = scan_chain(&lines);
        assert_eq!(result.verified_records.len(), 4);
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::LinkBroken {
                at_input_index: 4,
                error: ChainLinkError::BrokenPrevHash
            })
        );
        assert!(!may_resume_without_recovery(&result));
    }

    #[test]
    fn scan_chain_detects_a_sequence_gap() {
        let mut records = valid_chain(5);
        records[3].sequence = 99;
        // Recompute hash so BrokenRecordHash doesn't mask the gap we're testing.
        records[3].record_hash =
            compute_record_hash(&records[3].prev_record_hash, 99, &records[3].content_bytes);
        let lines: Vec<LineOutcome> = records.iter().cloned().map(LineOutcome::Parsed).collect();
        let result = scan_chain(&lines);
        assert_eq!(result.verified_records.len(), 3);
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::LinkBroken {
                at_input_index: 3,
                error: ChainLinkError::UnexpectedSequence {
                    expected: 3,
                    found: 99
                }
            })
        );
    }

    #[test]
    fn scan_chain_detects_a_duplicate_sequence_number() {
        let mut records = valid_chain(5);
        records[3].sequence = 2; // duplicate of records[2]'s own sequence
        records[3].record_hash =
            compute_record_hash(&records[3].prev_record_hash, 2, &records[3].content_bytes);
        let lines: Vec<LineOutcome> = records.iter().cloned().map(LineOutcome::Parsed).collect();
        let result = scan_chain(&lines);
        assert_eq!(result.verified_records.len(), 3);
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::LinkBroken {
                at_input_index: 3,
                error: ChainLinkError::UnexpectedSequence {
                    expected: 3,
                    found: 2
                }
            })
        );
    }

    #[test]
    fn scan_chain_reordered_records_are_detected_as_a_broken_link_not_silently_accepted() {
        let records = valid_chain(5);
        let mut reordered = records.clone();
        reordered.swap(2, 3); // records 2 and 3 swapped -- sequence 3 now appears before sequence 2's own slot
        let lines: Vec<LineOutcome> = reordered.iter().cloned().map(LineOutcome::Parsed).collect();
        let result = scan_chain(&lines);
        assert_eq!(
            result.verified_records.len(),
            2,
            "only records 0 and 1 (untouched by the swap) remain trusted"
        );
        assert!(matches!(
            result.stop_reason,
            Some(ScanStopReason::LinkBroken {
                at_input_index: 2,
                ..
            })
        ));
        assert!(!may_resume_without_recovery(&result));
    }

    // --- scan_chain: unparseable lines, tail vs. not-tail ---

    #[test]
    fn scan_chain_classifies_a_true_trailing_unparseable_line_as_partial_trailing_record() {
        let records = valid_chain(5);
        let mut lines: Vec<LineOutcome> =
            records.iter().cloned().map(LineOutcome::Parsed).collect();
        lines.push(LineOutcome::Unparseable {
            is_physical_tail: true,
        });
        let result = scan_chain(&lines);
        assert_eq!(
            result.verified_records.len(),
            5,
            "the full valid prefix must remain trusted"
        );
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::PartialTrailingRecord)
        );
        assert!(!may_resume_without_recovery(&result), "a partial trailing record must require an explicit recovery procedure, never silent resumption");
    }

    #[test]
    fn scan_chain_classifies_an_unparseable_line_followed_by_more_lines_as_not_at_tail_even_if_marked_tail(
    ) {
        // Defense in depth: even if a caller mistakenly marks a
        // not-truly-last line as `is_physical_tail: true`, the scan's own
        // position-in-input check (`index == lines.len() - 1`) overrides
        // it -- this is deliberately NOT solely trusting the caller's flag.
        let records = valid_chain(3);
        let mut lines: Vec<LineOutcome> =
            records.iter().cloned().map(LineOutcome::Parsed).collect();
        lines.push(LineOutcome::Unparseable {
            is_physical_tail: true,
        }); // mismarked
        lines.push(LineOutcome::Parsed(record(
            3,
            &records[2].record_hash,
            b"after-garbage",
        )));
        let result = scan_chain(&lines);
        assert_eq!(result.stop_reason, Some(ScanStopReason::UnparseableNotAtTail), "a line with more input after it must never be classified as the physical tail, regardless of its own flag");
    }

    #[test]
    fn scan_chain_classifies_a_mid_file_unparseable_line_as_unparseable_not_at_tail() {
        let records = valid_chain(3);
        let mut lines: Vec<LineOutcome> =
            records.iter().cloned().map(LineOutcome::Parsed).collect();
        lines.insert(
            2,
            LineOutcome::Unparseable {
                is_physical_tail: false,
            },
        );
        let result = scan_chain(&lines);
        assert_eq!(result.verified_records.len(), 2);
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::UnparseableNotAtTail)
        );
        assert!(!may_resume_without_recovery(&result));
    }

    #[test]
    fn scan_chain_on_a_single_unparseable_line_with_nothing_before_it_is_still_a_partial_trailing_record_not_a_crash(
    ) {
        // The very first record ever written could itself be interrupted
        // mid-write on a brand-new, otherwise-empty log -- this must be
        // handled identically to a partial tail after a long valid
        // history, not treated as a special/different case.
        let lines = vec![LineOutcome::Unparseable {
            is_physical_tail: true,
        }];
        let result = scan_chain(&lines);
        assert_eq!(result.verified_records, vec![]);
        assert_eq!(result.last_verified, None);
        assert_eq!(
            result.stop_reason,
            Some(ScanStopReason::PartialTrailingRecord)
        );
        assert!(!may_resume_without_recovery(&result));
    }
}
