# B4.3 — Vector Index Storage Format

**Status: format frozen before implementation.**

## 1. Location

`.superbrain-model-cache/dense-index/<project>.bin` at the vault root
(B4.1 decision: `.superbrain-model-cache/` mirrors the existing
`.pii-store/` vault-root convention) — one file per project, since B1's
own candidate generation is already project-scoped
(`search.rs`'s `ProjectScope`/`active_project`).

## 2. On-disk layout (binary, versioned)

```
[4 bytes]  magic: b"SBDI" (SuperBrain Dense Index)
[4 bytes]  format_version: u32 LE  (starts at 1 -- independent of model
                                     identity; a future storage-format
                                     change bumps this, triggering a
                                     rebuild distinct from a model
                                     change, per the design doc's own
                                     "Schema/versioning" requirement)
[32 bytes] model_identity_hash: SHA-256 of
             "{HF_REPO}|{PINNED_REVISION_SHA}|{ONNX_ARTIFACT_SHA256}|{TOKENIZER_ARTIFACT_SHA256}"
             (one combined hash of all four identity fields, rather
             than four separate fields, so a mismatch in ANY of them
             is a single comparison -- simpler than reproducing
             frozen-config-v2.json's own four-field structure on disk)
[4 bytes]  embedding_dimension: u32 LE  (384, but stored and checked,
                                         not assumed, so a future model
                                         with a different dimension
                                         fails closed rather than
                                         corrupting reads)
[4 bytes]  note_count: u32 LE
[4 bytes]  checksum: u32 LE  (CRC32 of every byte that follows this
                               field -- corruption detection, per the
                               design doc's own "Corruption detection"
                               requirement; a stored checksum, checked
                               against a freshly computed one on load)
-- repeated note_count times --
[2 bytes]  note_id_len: u16 LE  (NoteId is capped at 128 bytes,
                                  MAX_NOTE_ID_LEN in storage::note_id,
                                  so u16 is already more headroom than
                                  needed, chosen over u8 only for
                                  explicit future-proofing margin)
[note_id_len bytes]  note_id: UTF-8 bytes, NOT null-terminated (length-
                               prefixed, so a NoteId's own byte content
                               can never be misread as a terminator)
[8 bytes]  content_hash: u64 LE  (a fast, non-cryptographic hash --
                                   e.g. a 64-bit FNV or similar -- of
                                   the note's text_searchable AT THE
                                   TIME it was embedded; this is the
                                   "note's own stored hash" the design
                                   doc's incremental-update rule
                                   compares against to decide whether
                                   to re-embed. NOT a security
                                   boundary, so a non-cryptographic
                                   hash is correct and faster than
                                   SHA-256 here.)
384 * 4 bytes  embedding: 384 f32 values, LE, L2-normalized (the
                           EMBEDDING_DIMENSION field above states the
                           real count; 384 is today's value)
```

**Why a custom binary format, not an existing serialization
library:** matches this project's own existing convention —
`storage`'s own on-disk record format
(`vault.rs`'s `serialize_record`) is also a hand-rolled, versioned
binary layout, not `serde`+`bincode` or similar, for the same reason:
full control over exactly what bytes land on disk, with no
serialization-library version/feature-flag surface to track as its
own dependency. This storage format follows that same discipline
rather than introducing a new one.

**Why CRC32 for the checksum, not SHA-256:** this is corruption
detection (did the write complete, did a bit flip), not a security
boundary (unlike the artifact hash verification in B4.2, which IS a
security/provenance check against a known-good value) — CRC32 is the
conventional, fast choice for this specific job, and using SHA-256
here would not add any real protection this file's threat model needs.

## 3. Atomic write

Mirrors `storage::atomic::atomic_write`'s own pattern exactly (not
reused directly — that function is `pub(crate)` to `storage`, per its
own doc comment, deliberately not exported): write the full buffer to
a sibling temp file in the same directory, `fsync`, then
`std::fs::rename` over the final path. A directory-entry fsync on Unix
only (same platform-specific reasoning `atomic_write`'s own doc
comment already gives for Windows' lack of reliable directory fsync
support).

## 4. Load-time validation sequence

1. Read the file. A missing file → "not yet built" (B4.0's frozen
   fallback condition 1).
2. Check `magic` → wrong magic is treated identically to "corrupted"
   (condition 3 below), not a separate case — a file that doesn't even
   start with the right magic bytes is not a format-version
   disagreement, it is not this format at all.
3. Check `checksum` against a freshly computed CRC32 of the remaining
   bytes → mismatch is "corrupted" → triggers rebuild (B4.4's job),
   never served.
4. Check `format_version` → unrecognized version triggers rebuild
   (distinct trigger from model-identity mismatch, per the design
   doc's own requirement that these stay distinguishable).
5. Check `model_identity_hash` against the currently-configured
   model's own combined hash → mismatch triggers full rebuild (B4.0's
   frozen fallback condition 2).
6. Check `embedding_dimension` against `EMBEDDING_DIMENSION` → mismatch
   triggers rebuild (same class as model-identity: the vectors are not
   usable as-is).

Any validation failure above results in the SAME outcome from
`DenseScorer::score`'s point of view: `None` (B4.0's frozen contract)
— rebuild is a separate, explicit operation (B4.4), never triggered
implicitly just because a read failed.

## 5. STOP — format frozen, implementation follows
