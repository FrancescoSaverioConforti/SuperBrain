//! M6.2B: extracted from `brain-api::audit_record` (B5 remediation M5
//! Task 4's concrete, versioned, newline-delimited line format). The
//! ONE change from the original: `serialize_record`/`parse_event` are
//! generalized from a hardcoded `brain_api::AuditEvent` type to any
//! `T: Serialize`/`T: DeserializeOwned` -- a behavior-preserving
//! generalization (both functions only ever touched `AuditEvent` through
//! `serde_json::to_string`/`serde_json::from_slice`, never anything
//! `AuditEvent`-specific), required so a future MCP Client audit payload
//! type can use this same record format without depending on
//! `brain-api` for its own event schema (M6.2B requirement 6). For the
//! one concrete type that calls it today (`brain_api::AuditEvent`, via
//! `brain-api`'s own thin re-export wrappers), the produced bytes and
//! hashes are byte-for-byte identical to the pre-extraction
//! implementation -- proven by M6.2A's golden-vector suite, run
//! unchanged against this extracted code.
//!
//! **The byte-identity requirement this module exists to satisfy**:
//! [`crate::chain::compute_record_hash`]/`verify_record_hash` hash
//! exactly `ChainRecord::content_bytes` -- raw bytes, never a value this
//! module reparses and reserializes. If this module's own `parse_line`
//! extracted a typed event, re-serialized IT, and hashed that instead of
//! the original bytes, a byte-identical-but-differently-ordered (or
//! differently-whitespaced) persisted record could silently verify
//! against the WRONG hash, or a genuinely tampered record could silently
//! re-serialize back to the same canonical form and defeat detection.
//! Both directions are closed by making `content_bytes` a self-contained,
//! independently-sliceable JSON STRING field (an embedded, escaped JSON
//! document, not a nested object) -- `parse_line` extracts this field's
//! raw escaped-string bytes once, decodes them via `serde_json` into an
//! owned `String` to pass to the hasher as the LITERAL CONTENT bytes
//! (never the surrounding envelope's own field order/whitespace), and the
//! typed value is only ever obtained by parsing that SAME decoded string
//! a second time, never the other way around. Verification and typed
//! access both start from the one identical byte source.
//!
//! **Format version**: every line a `serialize_record` call of this
//! version writes carries `format_version: 1`. `parse_line` rejects any
//! other value (or a missing one) with
//! [`AuditRecordParseError::UnsupportedFormatVersion`] rather than
//! guessing at forward/backward compatibility this module never
//! designed -- unchanged from the pre-extraction implementation.
//!
//! **Not this module's job**: deciding WHEN to append, computing
//! `prev_record_hash`/`sequence` (the caller owns the in-memory
//! chain-tail state and tells this module what the next record's
//! `sequence`/`prev_record_hash` are), or any filesystem/locking concern
//! at all. Pure, synchronous, allocates, never panics on malformed input
//! (returns a typed error).

use crate::chain::{compute_record_hash, ChainRecord};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// The only format version this module currently writes or accepts.
/// `parse_line` fails closed on anything else -- never silently
/// reinterprets an unknown version as this one.
pub const AUDIT_RECORD_FORMAT_VERSION: u32 = 1;

/// One line's own envelope, as persisted -- field order here IS the
/// on-disk field order (`serde_json`'s struct serialization is
/// deterministic by declaration order; this is a plain struct, never a
/// `serde_json::Value`/`Map`, so there is no key-ordering ambiguity to
/// begin with). `event_json` carries the embedded event document as an
/// ordinary JSON string value (escaped), never a nested object -- see
/// this module's own top-level doc comment for why.
#[derive(serde::Serialize, serde::Deserialize)]
struct RecordEnvelope {
    format_version: u32,
    sequence: u64,
    prev_record_hash: String,
    record_hash: String,
    event_json: String,
}

/// Why a line could not be parsed into a [`ChainRecord`] -- distinguished
/// precisely so a caller (the worker's startup recovery scan) can tell
/// "this line is not even well-formed JSON" apart from "this line is
/// well-formed JSON but names a format version this code does not
/// understand," neither of which this module silently papers over.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditRecordParseError {
    #[error("line is not valid JSON: {0}")]
    MalformedJson(String),
    #[error("record declares format_version {found}, this code only accepts {AUDIT_RECORD_FORMAT_VERSION}")]
    UnsupportedFormatVersion { found: u32 },
    #[error("event_json field is not valid JSON: {0}")]
    MalformedEventJson(String),
}

/// Serializes one event value into the exact bytes that must be both
/// (a) hashed via [`crate::chain::compute_record_hash`] as this record's
/// `content_bytes` and (b) written as this log line's trailing
/// newline-terminated bytes. Returns the full line bytes (including the
/// trailing `\n`) AND the `record_hash` the caller must pass to a
/// durable writer alongside them -- computed here, once, from the same
/// `content_bytes` this function itself derives, so a caller can never
/// accidentally hash something other than what gets persisted.
///
/// `sequence`/`prev_record_hash` are supplied by the caller (the
/// worker's own in-memory chain-tail state) -- this function does not
/// track or validate chain continuity itself; that is `scan_chain`'s job
/// on read-back, not this function's job on write.
pub fn serialize_record<T: Serialize>(event: &T, sequence: u64, prev_record_hash: &str) -> (Vec<u8>, String) {
    // The content bytes a verifier will hash: the event's OWN canonical
    // JSON serialization, independent of the envelope's own field names
    // or ordering. Computed once, reused for both the hash and the
    // embedded string field below -- never regenerated a second time
    // from a reparsed value.
    let event_json = serde_json::to_string(event).expect("event serialization cannot fail: no non-finite floats, no non-string map keys, no unbounded recursion");
    let record_hash = compute_record_hash(prev_record_hash, sequence, event_json.as_bytes());

    let envelope = RecordEnvelope {
        format_version: AUDIT_RECORD_FORMAT_VERSION,
        sequence,
        prev_record_hash: prev_record_hash.to_string(),
        record_hash: record_hash.clone(),
        event_json,
    };
    let mut line = serde_json::to_vec(&envelope).expect("RecordEnvelope serialization cannot fail: all fields are plain strings/integers");
    line.push(b'\n');
    (line, record_hash)
}

/// Parses one already-newline-stripped line's bytes into a [`ChainRecord`]
/// whose `content_bytes` are the EXACT bytes [`serialize_record`] hashed
/// (the decoded `event_json` string's own UTF-8 bytes) -- never a
/// re-serialization of a parsed event. A caller that also wants the
/// typed event back calls [`parse_event`] on `record.content_bytes`
/// separately; this function does not do that parse itself, keeping
/// chain verification's own byte source (`content_bytes`) independent of
/// whether the event's own schema can even be deserialized by the
/// current code (a future, unknown event variant added by a newer
/// version could still verify its chain hash correctly here even if it
/// cannot be decoded into a typed value).
pub fn parse_line(line: &[u8]) -> Result<ChainRecord, AuditRecordParseError> {
    let envelope: RecordEnvelope = serde_json::from_slice(line).map_err(|e| AuditRecordParseError::MalformedJson(e.to_string()))?;
    if envelope.format_version != AUDIT_RECORD_FORMAT_VERSION {
        return Err(AuditRecordParseError::UnsupportedFormatVersion { found: envelope.format_version });
    }
    // Validate `event_json` is at least well-formed JSON (never decoded
    // into a typed value here -- see this function's own doc comment on
    // why typed decoding is kept separate from chain verification).
    serde_json::from_str::<serde_json::Value>(&envelope.event_json).map_err(|e| AuditRecordParseError::MalformedEventJson(e.to_string()))?;

    Ok(ChainRecord {
        sequence: envelope.sequence,
        prev_record_hash: envelope.prev_record_hash,
        record_hash: envelope.record_hash,
        content_bytes: envelope.event_json.into_bytes(),
    })
}

/// Decodes a verified [`ChainRecord`]'s own `content_bytes` into a typed
/// value `T` -- kept separate from [`parse_line`] (see that function's
/// own doc comment): this is the ONE place a caller reconstructs the
/// typed event, only ever called AFTER
/// `crate::chain::verify_link`/`scan_chain` has already confirmed
/// `content_bytes` is exactly what the hash covers, never before.
pub fn parse_event<T: DeserializeOwned>(content_bytes: &[u8]) -> Result<T, AuditRecordParseError> {
    serde_json::from_slice(content_bytes).map_err(|e| AuditRecordParseError::MalformedEventJson(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::GENESIS_HASH;
    use serde::{Deserialize, Serialize};

    // A minimal stand-in event type for this crate's own tests -- this
    // crate has no concept of `AuditEventKind`/`AuditEvent` (those stay
    // in brain-api per M6.2B requirement 5); these tests only need SOME
    // `Serialize + DeserializeOwned + PartialEq` type to exercise the
    // format's own generic behavior.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestEvent {
        kind: String,
        value: u32,
    }

    fn sample_event() -> TestEvent {
        TestEvent { kind: "sample".to_string(), value: 42 }
    }

    // --- deterministic serialization ---

    #[test]
    fn serializing_the_same_event_twice_produces_byte_identical_lines() {
        let event = sample_event();
        let (line_a, hash_a) = serialize_record(&event, 0, GENESIS_HASH);
        let (line_b, hash_b) = serialize_record(&event, 0, GENESIS_HASH);
        assert_eq!(line_a, line_b, "serialization must be deterministic for identical input");
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn serialized_line_ends_with_exactly_one_newline() {
        let event = sample_event();
        let (line, _) = serialize_record(&event, 0, GENESIS_HASH);
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(line.iter().filter(|&&b| b == b'\n').count(), 1);
    }

    // --- byte-exact verification: round trip must preserve the exact
    //     hashed content_bytes, never a reserialization ---

    #[test]
    fn round_tripping_a_line_preserves_the_exact_hashed_content_bytes() {
        let event = sample_event();
        let (line, record_hash) = serialize_record(&event, 0, GENESIS_HASH);
        let without_newline = &line[..line.len() - 1];
        let record = parse_line(without_newline).unwrap();
        assert_eq!(record.record_hash, record_hash);
        assert_eq!(record.sequence, 0);
        assert_eq!(record.prev_record_hash, GENESIS_HASH);

        // The verification-critical property: re-hashing content_bytes
        // exactly as parsed must reproduce the same record_hash --
        // proving content_bytes is the SAME bytes that were hashed at
        // write time, not a reserialization.
        let rehash = compute_record_hash(&record.prev_record_hash, record.sequence, &record.content_bytes);
        assert_eq!(rehash, record.record_hash);
    }

    #[test]
    fn parse_event_recovers_the_original_typed_event_from_a_verified_records_content_bytes() {
        let event = sample_event();
        let (line, _) = serialize_record(&event, 0, GENESIS_HASH);
        let record = parse_line(&line[..line.len() - 1]).unwrap();
        let recovered: TestEvent = parse_event(&record.content_bytes).unwrap();
        assert_eq!(recovered, event);
    }

    #[test]
    fn a_chain_of_records_links_correctly_through_compute_record_hash() {
        let e0 = sample_event();
        let (_, hash0) = serialize_record(&e0, 0, GENESIS_HASH);
        let e1 = TestEvent { kind: "second".to_string(), value: 1 };
        let (line1, hash1) = serialize_record(&e1, 1, &hash0);
        let record1 = parse_line(&line1[..line1.len() - 1]).unwrap();
        assert_eq!(record1.prev_record_hash, hash0);
        assert_eq!(record1.record_hash, hash1);
    }

    // --- malformed records ---

    #[test]
    fn parse_line_rejects_bytes_that_are_not_valid_json() {
        let result = parse_line(b"not json at all {{{");
        assert!(matches!(result, Err(AuditRecordParseError::MalformedJson(_))), "got {result:?}");
    }

    #[test]
    fn parse_line_rejects_a_truncated_json_object() {
        let event = sample_event();
        let (line, _) = serialize_record(&event, 0, GENESIS_HASH);
        let truncated = &line[..line.len() / 2];
        let result = parse_line(truncated);
        assert!(matches!(result, Err(AuditRecordParseError::MalformedJson(_))), "got {result:?}");
    }

    #[test]
    fn parse_line_rejects_a_well_formed_envelope_with_malformed_event_json() {
        let json = r#"{"format_version":1,"sequence":0,"prev_record_hash":"x","record_hash":"y","event_json":"not valid json"}"#;
        let result = parse_line(json.as_bytes());
        assert!(matches!(result, Err(AuditRecordParseError::MalformedEventJson(_))), "got {result:?}");
    }

    // --- unsupported format version ---

    #[test]
    fn parse_line_rejects_an_unsupported_format_version() {
        let json = r#"{"format_version":2,"sequence":0,"prev_record_hash":"x","record_hash":"y","event_json":"{}"}"#;
        let result = parse_line(json.as_bytes());
        assert!(matches!(result, Err(AuditRecordParseError::UnsupportedFormatVersion { found: 2 })), "got {result:?}");
    }

    #[test]
    fn parse_line_rejects_a_missing_format_version_field() {
        let json = r#"{"sequence":0,"prev_record_hash":"x","record_hash":"y","event_json":"{}"}"#;
        let result = parse_line(json.as_bytes());
        assert!(matches!(result, Err(AuditRecordParseError::MalformedJson(_))), "a missing required field must fail JSON structural parsing, got {result:?}");
    }

    // --- JSON representation ambiguity (duplicate keys) ---
    //
    // Verified directly (not assumed): `serde_json`'s DERIVE-based struct
    // deserialization (the envelope struct here, and `T` itself in
    // `parse_event`) rejects a duplicate key outright as malformed --
    // confirmed by `parse_line_rejects_a_duplicate_envelope_key_outright`
    // below. But `parse_line`'s own `event_json` structural-validity
    // check deserializes into `serde_json::Value` (an arbitrary JSON
    // tree, not a derived struct) to avoid depending on `T`'s own schema
    // for that one check -- and `serde_json::Value` resolves a duplicate
    // key as ordinary last-value-wins (confirmed by direct probe:
    // `serde_json::from_str::<Value>(r#"{"a":1,"a":2}"#)` returns
    // `Ok(Object {"a": 2})`, not an error). This means a duplicate key
    // INSIDE `event_json` survives `parse_line`'s own check -- the
    // SECOND test below proves the later, decisive gate is
    // `parse_event`'s own struct-typed deserialization, which DOES
    // reject it, so no ambiguous `content_bytes` ever actually reaches a
    // caller as two different valid typed interpretations, even though
    // `parse_line` alone is not the layer that catches it.

    #[test]
    fn parse_line_rejects_a_duplicate_envelope_key_outright() {
        let json = r#"{"format_version":1,"format_version":1,"sequence":0,"prev_record_hash":"x","record_hash":"y","event_json":"{}"}"#;
        let result = parse_line(json.as_bytes());
        assert!(matches!(result, Err(AuditRecordParseError::MalformedJson(_))), "a duplicate envelope key must be rejected outright, never silently resolved to either value, got {result:?}");
    }

    #[test]
    fn a_duplicate_key_inside_event_json_survives_parse_line_but_parse_event_rejects_it() {
        let inner = r#"{"kind":"a","kind":"b","value":1}"#;
        let escaped_inner = inner.replace('\\', "\\\\").replace('"', "\\\"");
        let json = format!(r#"{{"format_version":1,"sequence":0,"prev_record_hash":"x","record_hash":"y","event_json":"{escaped_inner}"}}"#);

        // parse_line's own check (Value-level) does not reject this --
        // documented, verified behavior, not a gap this module silently
        // papers over.
        let record = parse_line(json.as_bytes()).expect("parse_line's Value-level check accepts a duplicate key via last-wins, verified directly");

        // The decisive rejection happens one layer later, at
        // parse_event's own struct-typed deserialization.
        let result: Result<TestEvent, _> = parse_event(&record.content_bytes);
        assert!(result.is_err(), "parse_event must reject a duplicate key in the typed value, never silently resolve it, got {result:?}");
    }
}
