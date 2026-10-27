# A1 — Schema e storage (Fase A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `domain`, `storage`, and `policy` Rust crates that validate a note's schema, enforce the classification/secret gate, and write notes to the filesystem atomically with explicit optimistic-concurrency conflicts — with no SQLite journal and no git integration (those are A2).

**Architecture:** Three-crate Cargo workspace with crate dependencies revised during A1 from the spec's original table — see the note below — to: `domain` has no internal dependencies; `policy` depends only on `domain`; `storage` depends on `domain` and `policy`. `domain` holds pure types (no I/O): `Note`, schema validation, link states, a generic schema-migration mechanism, and a `WriteState` enum that mirrors the C1 state machine's *names* without implementing persistence or recovery. `policy` composes `domain` validation into a single `Gate::evaluate` decision (schema, links, classification/secret) and has no knowledge of storage. `storage` performs atomic file writes (temp file → fsync → rename → fsync dir) of a single content+version record and enforces optimistic concurrency (the expected-version check and the write happen inside one critical section — a per-note mutex, scoped to one `Vault` instance/process, not a cross-process guarantee); its raw write primitive is crate-private, and the only write function `storage` exposes publicly, `propose_and_commit`, runs `policy::Gate::evaluate` internally before ever touching the filesystem — making gate bypass a compile-time impossibility for any external caller, since the ungated primitive simply isn't reachable from outside the crate. **Dependency-table note**: the spec's original crate table (line 44) lists `storage` depending only on `domain`; A1's implementation found that orchestrating "validate then write" behind one public function requires the validating crate and the writing crate to share a dependency edge, and chose `storage → policy` (rather than `policy → storage`) so the write primitive — the thing that most needs to be inaccessible — stays privately owned by the crate that also holds the gate call. This is a one-line, implementation-driven amendment to the spec's table, not a new architectural layer. `storage`'s write function already takes an `operation_id` parameter (ignored/unused for recovery purposes in A1) so A2 can add journal persistence and crash recovery behind the same signature without changing `domain` or any caller.

**Tech Stack:** Rust (stable toolchain), Cargo workspace, `serde` + `serde_yaml` for frontmatter, `thiserror` for typed errors, `tempfile` not used directly (manual temp-file+rename for fsync control), `cargo-deny` for license/vuln policy, `cargo-mutants` for mutation testing, standard `#[test]` / `cargo test` (no async runtime needed — concurrency test uses `std::thread`).

**Spec:** `docs/Spec_mvp.md` (sections: "Fase A — Vault affidabile" consegne 1–2 and relevant test bullets; "Appendice contrattuale" C1 and C3; "Ordine di implementazione" item 1 defines A1's exact boundary as "validazione, scrittura concorrente e conflitti")

## Global Constraints

- Only dependencies with SPDX licenses in the explicit allowlist: Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib — verified via `cargo-deny`, checked against the producer's source per the spec.
- No secrets, no network calls in the `domain`/`storage`/`policy` crates — Fase A runs with zero AI model loaded.
- `classification: secret` is always rejected by the gate; it is a forbidden value in the vault, not merely discouraged.
- `classification: sensitive` is rejected by the gate in A1 (no encrypted `pii://` store exists yet — that's A3); the rejection must say why (points at A3, not a generic validation failure) so this isn't mistaken for a bug once A3 lands.
- No journal SQLite, no git commit, no crash-recovery state machine, no backup/restore, no encryption, no deletion registry in this plan — those are A2/A3. Any task that would need them stops at defining the interface shape (e.g., `operation_id: Uuid` parameter, `WriteState` enum variants) without implementing persistence for it.
- `storage`'s write path must perform the expected-version check and the actual write inside the same critical section (single mutex guarding a given note's write path) — this is the user's explicit constraint to prevent two concurrent writers both observing a stale "valid" version. This guarantee is scoped to callers sharing one `Vault` instance/process; it is not a cross-process or crash-safe guarantee (that's A2).
- Content and version are persisted as a single atomically-replaced unit (one file, one `atomic_write` call), never as two separate files — a crash or a concurrent reader must never observe content from one version paired with a version number from another.
- `storage::propose_and_commit` is the only `pub` write function `storage` exposes; the raw, ungated write primitive (`write_record_unchecked`) is `pub(crate)` — never plain `pub` — and therefore unreachable from any other crate including `policy`, no matter what. (Implementation note from Task 9: a bare, module-private `fn` cannot be called from a sibling module within the same crate, and `propose_and_commit` lives in its own file per this plan's required layout, so `pub(crate)` — not a bare-private `fn` — is the correct and only way to satisfy both "separate module" and "callable by `propose_and_commit`" while keeping the primitive unreachable from outside `storage`. This was verified by an independent reviewer who compiled a standalone Rust repro confirming the visibility semantics before approving the change.) `propose_and_commit` runs `policy::Gate::evaluate` internally before calling the private primitive — this is a hard compiler boundary, not a convention, because there is no other way to reach a filesystem write. `storage`'s crate dependency is revised from the spec's original table to `storage → domain, policy` (documented as a one-line amendment, see Architecture) so `storage` can call `policy::Gate::evaluate` directly; `policy` continues to depend only on `domain` and has no dependency on `storage`, so no cycle exists.
- `note_id` is validated before it is used to build any filesystem path: reject empty strings, path separators, `..` components, and absolute paths; additionally canonicalize and confirm the resolved path stays within the vault root (no symlink escape) before any file operation touches disk.
- Every typed error must name the offending field/condition (e.g., which frontmatter field failed, which link is broken, which classification value was rejected) — generic "validation failed" errors do not satisfy the spec's test bullets.
- A registered secret-policy exception covers only the exact matched occurrence it was granted for (pattern + note + hash of the matched text), never every future occurrence of the same pattern in the same note.
- No test may depend on wall-clock sleeps to coordinate the concurrency test; use explicit synchronization (barriers/channels) so the test is deterministic.
- Evidence for every acceptance criterion goes in `eval/results/A1/` (commit hash, command run, raw output, verdict) — a checkpoint is not done without this artifact.
- Do not implement index, agent, api, or model-related code — out of scope for every phase before B.
- A1 is not the transactional vault promised by the full spec: it must never be reported, in evidence or commit messages, as providing crash recovery or persistent cross-restart idempotency. Those guarantees are implemented and verified in A2.

## Review Focus

- **Frontmatter with an unknown/extra field, or a field of the wrong YAML type** (e.g., `created` as a number instead of a date string) — a reasonable implementer expects a named-field rejection, not a generic deserialization panic or silent coercion. Covered in Task 2.
- **A note whose `relations.supports` etc. list contains a self-reference (id lists itself)** — the spec doesn't ban this explicitly, but an unvalidated self-link would corrupt the dependency-propagation logic that Fase C builds on `derived_from`/`supports`. Covered in Task 3 as a `resolved`-vs-cycle edge case documented (not fully solved — Fase C's responsibility — but A1 must not silently accept a structurally nonsensical self-link as `resolved` without at least flagging it).
- **Classification value that is neither `public`, `internal`, `sensitive`, nor `secret`** (typo, old schema value) — must be a named-field validation error, not treated as implicitly `public`. Covered in Task 2.
- **Two concurrent writers where one retries with a *new* expected version after losing the first conflict** — a reasonable user expects the retry to succeed once it reads the current version, not to hit a stale mutex or deadlock. Covered in Task 6 as a second scenario alongside the basic conflict test.
- **A write attempt with a expected_version that matches a *different* note's current version by coincidence** (version numbers are small integers, collision across different notes is easy) — the version check must be scoped per-note-id, not a global counter comparison. Covered in Task 6 by using distinct note ids with overlapping version numbers in the test fixtures.
- **A `note_id` crafted to escape the vault root** (`../..`, absolute paths, or a symlink planted inside the vault pointing outside it) — a reasonable implementer expects this rejected before any filesystem call, not after a write lands outside the intended directory. Covered in Task 6.

---

## File Structure

```
SuperBrain/
├── Cargo.toml                          # workspace root, members = ["crates/domain","crates/policy","crates/storage"]
├── crates/
│   ├── domain/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # re-exports
│   │       ├── note.rs                 # Note, NoteMetadata, NoteType, NoteStatus, TrustLevel, Classification
│   │       ├── schema.rs               # SchemaVersion, frontmatter parse/validate, ValidationError
│   │       ├── links.rs                # LinkState, Relations, link validation
│   │       ├── migration.rs            # generic Migration trait, MigrationOutcome<T>, proven on a test-only fixture
│   │       └── write_state.rs          # WriteState enum (names only, per C1), no persistence
│   ├── policy/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── gate.rs                  # Gate, GateDecision, GateRejection
│   │       └── secrets.rs               # secret-pattern policy, occurrence-scoped exceptions
│   └── storage/
│       ├── Cargo.toml                   # depends on domain AND policy (amended from the spec's original table)
│       └── src/
│           ├── lib.rs
│           ├── vault.rs                 # Vault, write_record_unchecked (pub(crate), never pub), read_note, current_version
│           ├── note_id.rs               # NoteId newtype: path-traversal/symlink-safe, lowercase-ASCII-only id validation
│           ├── atomic.rs                # temp-file+fsync+rename+fsync-dir helpers — atomic_write is pub(crate) and NOT re-exported (final-review fix C1)
│           └── propose.rs               # propose_and_commit: the ONLY pub write function in the whole workspace (atomic_write and write_record_unchecked are both pub(crate); pinned by compile_fail doctests in storage/src/lib.rs)
├── eval/
│   └── results/
│       └── A1/                          # evidence artifacts, one file per checkpoint
└── docs/
    └── superpowers/plans/2026-10-26-a1-schema-storage.md   # this file
```

---

## Task 1: Workspace scaffolding and CI

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/domain/Cargo.toml`, `crates/domain/src/lib.rs`
- Create: `crates/storage/Cargo.toml`, `crates/storage/src/lib.rs`
- Create: `crates/policy/Cargo.toml`, `crates/policy/src/lib.rs`
- Create: `deny.toml` (cargo-deny config: license allowlist)
- Create: `.github/workflows/ci.yml` (or equivalent CI config if the repo uses a different CI; check for existing `.github` first)

**Interfaces:**
- Produces: three empty library crates — `domain` (no path dependencies), `policy` (depends on `domain`), `storage` (depends on `domain` and `policy` — this is a one-line amendment to the spec's original crate table, made so `storage` can later own both the gate call and the filesystem write behind a single public function; see the plan's Architecture section) — that build and have one placeholder test each, plus a `deny.toml` with the exact allowlist: `Apache-2.0`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`.

- [ ] **Step 1: Check for existing CI config**

Run: `ls -la .github/workflows 2>/dev/null; cat README.md`
Expected: confirms no existing CI/workspace (per repo inspection already done — only `README.md` and `docs/` exist).

- [ ] **Step 2: Create workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/domain", "crates/policy", "crates/storage"]
```

- [ ] **Step 3: Create `crates/domain/Cargo.toml`, `crates/policy/Cargo.toml`, `crates/storage/Cargo.toml`**

`domain`: no path dependencies. `policy`: `domain = { path = "../domain" }` only. `storage`: `domain = { path = "../domain" }` and `policy = { path = "../policy" }`. Each crate: `edition = "2021"`, add `serde = { version = "1", features = ["derive"] }`, `serde_yaml = "0.9"`, `thiserror = "1"` to `domain`.

- [ ] **Step 4: Create placeholder `lib.rs` in each crate with one trivial passing test**

```rust
#[test]
fn crate_builds() { assert!(true); }
```

- [ ] **Step 5: Run `cargo build --workspace` and `cargo test --workspace`**

Run: `cargo build --workspace && cargo test --workspace`
Expected: builds clean, 3 tests pass (one per crate).

- [ ] **Step 6: Create `deny.toml` with the exact SPDX allowlist**

```toml
[licenses]
allow = ["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib"]
```

- [ ] **Step 7: Install and run cargo-deny**

Run: `cargo install cargo-deny --locked` (if not present), then `cargo deny check licenses`
Expected: PASS with no violations (only serde/serde_yaml/thiserror at this point — verify each is MIT or Apache-2.0 per crates.io).

- [ ] **Step 8: Create minimal CI workflow running build, test, clippy, deny**

```yaml
name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --workspace
      - run: cargo test --workspace
      - run: cargo clippy --workspace -- -D warnings
      - run: cargo install cargo-deny --locked
      - run: cargo deny check
```

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/ deny.toml .github/workflows/ci.yml
git commit -m "chore: scaffold domain/storage/policy workspace and CI"
```

---

## Task 2: Note schema and frontmatter validation (domain)

**Files:**
- Create: `crates/domain/src/note.rs`
- Create: `crates/domain/src/schema.rs`
- Modify: `crates/domain/src/lib.rs` (add `mod note; mod schema; pub use ...`)
- Test: inline `#[cfg(test)]` modules in `note.rs` and `schema.rs`

**Interfaces:**
- Produces:
  - `pub struct NoteMetadata { id: String, schema_version: u32, r#type: NoteType, status: NoteStatus, trust: TrustLevel, classification: Classification, project: String, source: String, created: String, updated: String, event_period: String, relations: Relations }`
  - `pub enum NoteType { Code, Bug, Decision, Pattern, Research, Session, Summary }`
  - `pub enum NoteStatus { Draft, TestPassed, Reviewed, Verified, Superseded, Contradicted }`
  - `pub enum TrustLevel { Web, Internal, Official }` (ordered: `Web < Internal < Official` — implement `Ord`)
  - `pub enum Classification { Public, Internal, Sensitive }` — **note: no `Secret` variant.** `secret` from YAML parses to a dedicated `ValidationError::SecretClassificationForbidden` at parse time, not as an enum value, because the spec says a `secret`-classified note is *always* rejected — it must never become a valid in-memory `Classification`.
  - `pub enum ValidationError { MissingField(String), InvalidFieldType { field: String, expected: String }, InvalidEnumValue { field: String, value: String }, SecretClassificationForbidden, UnknownField(String) }` (derive `thiserror::Error`). `UnknownField` was added by A1 final-review fix I1: any top-level frontmatter key outside the fixed field set, and any key inside `relations` other than the five relation names, is rejected by name (`"totally_unknown"`, `"relations.supprots"`) instead of being silently ignored; `Relations` also carries `#[serde(deny_unknown_fields)]`.
  - `pub fn parse_frontmatter(yaml: &str) -> Result<NoteMetadata, ValidationError>`
- Consumes: nothing (first domain task after scaffolding).

- [ ] **Step 1: Write failing tests for valid and invalid frontmatter**

```rust
#[test]
fn valid_note_is_accepted() {
    let yaml = r#"
id: bug-2026-10-23-fastapi-cors
schema_version: 1
type: bug
status: draft
trust: internal
classification: internal
project: project-x
source: agent:coder@prompt-v3
created: 2026-10-23
updated: 2026-10-23
event_period: 2026-10-23
relations:
  supports: []
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
"#;
    let note = parse_frontmatter(yaml).unwrap();
    assert_eq!(note.id, "bug-2026-10-23-fastapi-cors");
}

#[test]
fn missing_field_names_the_field() {
    let yaml = "id: x\nschema_version: 1\n"; // missing type, status, etc.
    let err = parse_frontmatter(yaml).unwrap_err();
    match err {
        ValidationError::MissingField(f) => assert_eq!(f, "type"),
        other => panic!("expected MissingField(type), got {other:?}"),
    }
}

#[test]
fn secret_classification_is_always_rejected() {
    let yaml = base_yaml_with_classification("secret");
    let err = parse_frontmatter(&yaml).unwrap_err();
    assert!(matches!(err, ValidationError::SecretClassificationForbidden));
}

#[test]
fn unknown_classification_value_is_named_field_error() {
    let yaml = base_yaml_with_classification("top-secret");
    let err = parse_frontmatter(&yaml).unwrap_err();
    match err {
        ValidationError::InvalidEnumValue { field, value } => {
            assert_eq!(field, "classification");
            assert_eq!(value, "top-secret");
        }
        other => panic!("expected InvalidEnumValue, got {other:?}"),
    }
}

#[test]
fn wrong_type_for_created_is_named_field_error() {
    let yaml = base_yaml_with_created_as_number(); // created: 20261023 (int, not string)
    let err = parse_frontmatter(&yaml).unwrap_err();
    match err {
        ValidationError::InvalidFieldType { field, .. } => assert_eq!(field, "created"),
        other => panic!("expected InvalidFieldType(created), got {other:?}"),
    }
}
```

(Helper functions `base_yaml_with_classification` / `base_yaml_with_created_as_number` build on the valid template above, swapping one field — write these as private test helpers in the same module.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p domain parse_frontmatter`
Expected: FAIL — `parse_frontmatter` not defined.

- [ ] **Step 3: Implement `NoteMetadata`, enums, `ValidationError`, and `parse_frontmatter` in `note.rs`/`schema.rs`**

Parse with `serde_yaml` into an intermediate struct using `String`/`serde_yaml::Value` for enum-typed fields so you can produce named-field errors instead of serde's generic deserialization error; validate field presence and enum values by hand after the raw parse, mapping to the specific `ValidationError` variant. `TrustLevel` derives `PartialOrd, Ord` in declaration order `Web, Internal, Official` (Rust derives `Ord` by variant declaration order, which matches the spec's required ordering `web < internal < official`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p domain parse_frontmatter`
Expected: PASS, all 5 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/domain/src/note.rs crates/domain/src/schema.rs crates/domain/src/lib.rs
git commit -m "feat(domain): note schema parsing with named-field validation errors"
```

---

## Task 3: Link states and relations validation (domain)

**Files:**
- Create: `crates/domain/src/links.rs`
- Modify: `crates/domain/src/lib.rs`
- Test: inline `#[cfg(test)]` in `links.rs`

**Interfaces:**
- Consumes: `NoteMetadata`, `Relations` from Task 2 (`relations: Relations` field with `supports: Vec<String>`, `contradicts: Vec<String>`, `derived_from: Vec<String>`, `supersedes: Vec<String>`, `applies_to: Vec<String>`).
- Produces:
  - `pub enum LinkState { Unlinked, Resolved, Pending, Broken }`
  - `pub enum LinkError { BrokenLink { relation: String, target_id: String }, SelfReference { relation: String, note_id: String } }`
  - `pub fn classify_links(note: &NoteMetadata, known_ids: &HashSet<String>, pending_ids: &HashSet<String>) -> Result<LinkState, LinkError>` — `known_ids` = ids that resolve to existing notes/symbols; `pending_ids` = ids explicitly recognized as external-source/not-yet-imported/unresolved-symbol (ammesso); any id in neither set that is referenced is `Broken`.

- [ ] **Step 1: Write failing tests for each link state and the self-reference edge case**

```rust
#[test]
fn broken_internal_link_is_rejected() {
    let note = note_with_relation("supports", "missing-note-id");
    let err = classify_links(&note, &HashSet::new(), &HashSet::new()).unwrap_err();
    assert!(matches!(err, LinkError::BrokenLink { .. }));
}

#[test]
fn pending_reference_is_accepted_and_flagged() {
    let note = note_with_relation("derived_from", "external-pending-id");
    let mut pending = HashSet::new();
    pending.insert("external-pending-id".to_string());
    let state = classify_links(&note, &HashSet::new(), &pending).unwrap();
    assert_eq!(state, LinkState::Pending);
}

#[test]
fn note_without_links_is_unlinked_and_accepted() {
    let note = note_with_no_relations();
    let state = classify_links(&note, &HashSet::new(), &HashSet::new()).unwrap();
    assert_eq!(state, LinkState::Unlinked);
}

#[test]
fn resolved_link_to_known_id_is_accepted() {
    let note = note_with_relation("supports", "existing-id");
    let mut known = HashSet::new();
    known.insert("existing-id".to_string());
    let state = classify_links(&note, &known, &HashSet::new()).unwrap();
    assert_eq!(state, LinkState::Resolved);
}

#[test]
fn self_reference_is_rejected_not_silently_resolved() {
    let note = note_with_relation("supports", "self-id"); // note.id == "self-id"
    let mut known = HashSet::new();
    known.insert("self-id".to_string());
    let err = classify_links(&note, &known, &HashSet::new()).unwrap_err();
    assert!(matches!(err, LinkError::SelfReference { .. }));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p domain classify_links`
Expected: FAIL — not defined.

- [ ] **Step 3: Implement `LinkState`, `LinkError`, `classify_links` in `links.rs`**

Check self-reference first (any relation list containing `note.id` itself → `SelfReference`), then: no relations at all → `Unlinked`; any id in `known_ids` → contributes `Resolved`; any id in `pending_ids` → contributes `Pending`; any id in neither → `Broken` (short-circuit, this is the bullet "broken rejected"). When a note has a mix of resolved and pending links (no broken), return `Pending` if any pending exists, else `Resolved` — document this precedence choice as a one-line comment since the spec doesn't pin the exact precedence rule for mixed states.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p domain classify_links`
Expected: PASS, all 5 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/domain/src/links.rs crates/domain/src/lib.rs
git commit -m "feat(domain): link state classification (unlinked/resolved/pending/broken)"
```

---

## Task 4: WriteState enum (domain, interface-only for A2)

**Files:**
- Create: `crates/domain/src/write_state.rs`
- Modify: `crates/domain/src/lib.rs`
- Test: inline `#[cfg(test)]` in `write_state.rs`

**Interfaces:**
- Produces:
  - `pub enum WriteState { IntentRecorded, ContentPersisted, JournalCommitted, GitCommitted, IndexCurrent, Recovering, Failed }`
  - `pub fn valid_transition(from: WriteState, to: WriteState) -> bool` — encodes the C1 legal ordering (`IntentRecorded → ContentPersisted → JournalCommitted → GitCommitted → IndexCurrent`, plus `Recovering` reachable from any non-terminal state and `Failed` reachable only from `IntentRecorded`, per C1's recovery table — `Failed` is terminal, no outgoing transitions).
- Consumes: nothing new. This type is **not wired into `storage` in A1** — it exists so A2 can import it without domain changes. Task 5/6's `storage` code does not use `WriteState` at all in A1.

- [ ] **Step 1: Write failing tests for the transition table**

```rust
#[test]
fn intent_recorded_can_advance_to_content_persisted() {
    assert!(valid_transition(WriteState::IntentRecorded, WriteState::ContentPersisted));
}

#[test]
fn intent_recorded_can_fail() {
    assert!(valid_transition(WriteState::IntentRecorded, WriteState::Failed));
}

#[test]
fn failed_is_terminal() {
    assert!(!valid_transition(WriteState::Failed, WriteState::IntentRecorded));
    assert!(!valid_transition(WriteState::Failed, WriteState::Recovering));
}

#[test]
fn cannot_skip_journal_committed_to_index_current() {
    assert!(!valid_transition(WriteState::ContentPersisted, WriteState::IndexCurrent));
}

#[test]
fn git_committed_can_reach_index_current() {
    assert!(valid_transition(WriteState::GitCommitted, WriteState::IndexCurrent));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p domain write_state`
Expected: FAIL — not defined.

- [ ] **Step 3: Implement `WriteState` and `valid_transition` in `write_state.rs`**

A straightforward `match (from, to)` table encoding exactly the C1 ordering plus the `Recovering`/`Failed` side-transitions described above.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p domain write_state`
Expected: PASS, all 5 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/domain/src/write_state.rs crates/domain/src/lib.rs
git commit -m "feat(domain): WriteState enum mirroring C1 state names (interface only, no persistence)"
```

---

## Task 5: Atomic file write primitive (storage)

**Files:**
- Create: `crates/storage/src/atomic.rs`
- Modify: `crates/storage/src/lib.rs`
- Test: inline `#[cfg(test)]` in `atomic.rs`, using `std::env::temp_dir()` + a per-test unique subdirectory (do not use a shared fixed path — tests must not collide when run in parallel)

**Interfaces:**
- Produces:
  - `pub(crate) enum AtomicWriteError { Io(std::io::Error) }` (derive `thiserror::Error`, `#[from] std::io::Error`)
  - `pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), AtomicWriteError>` — **crate-private, never re-exported** (corrected by the A1 final-review fix C1: it was originally written as `pub` and re-exported from `lib.rs`, which let any external crate write raw bytes to any path, bypassing `NoteId` validation, the gate, and the version check; `Vault` folds `AtomicWriteError` into `StorageError::Io` at the crate boundary). Writes to `<path>.tmp-<random>`, calls `File::sync_all()`, renames to `path`, then opens and syncs the parent directory.
- Consumes: nothing (pure filesystem helper).

- [ ] **Step 1: Write failing tests for atomic write correctness**

```rust
#[test]
fn atomic_write_produces_readable_file_with_exact_contents() {
    let dir = unique_test_dir();
    let path = dir.join("note.md");
    atomic_write(&path, b"hello world").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
}

#[test]
fn atomic_write_leaves_no_temp_file_behind_on_success() {
    let dir = unique_test_dir();
    let path = dir.join("note.md");
    atomic_write(&path, b"content").unwrap();
    let leftover: Vec<_> = std::fs::read_dir(&dir).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftover.is_empty());
}

#[test]
fn atomic_write_overwrites_existing_file_completely() {
    let dir = unique_test_dir();
    let path = dir.join("note.md");
    atomic_write(&path, b"first version, much longer content here").unwrap();
    atomic_write(&path, b"v2").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"v2"); // no trailing bytes from the longer first write
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p storage atomic_write`
Expected: FAIL — not defined.

- [ ] **Step 3: Implement `atomic_write` in `atomic.rs`**

Use `std::fs::File::create` on a sibling temp path (same directory as target, so rename is same-filesystem), write all bytes, `file.sync_all()`, drop the file handle, `std::fs::rename(tmp, path)`, then open the parent directory with `File::open(dir)` and call `.sync_all()` on it to flush the directory entry (on Windows, directory fsync is a no-op/unsupported in the std API — guard this with `#[cfg(unix)]` for the directory-sync step and document that Windows relies on `rename`'s own durability; note this platform gap explicitly in a code comment since the spec says "l'ordine esatto delle sincronizzazioni ... si fissa e si testa sulla piattaforma scelta").

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p storage atomic_write`
Expected: PASS, all 3 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/storage/src/atomic.rs crates/storage/src/lib.rs
git commit -m "feat(storage): atomic file write via temp+rename+fsync"
```

---

## Task 6: Vault write with optimistic concurrency and explicit conflicts (storage)

**Files:**
- Create: `crates/storage/src/vault.rs`
- Create: `crates/storage/src/note_id.rs`
- Modify: `crates/storage/src/lib.rs`
- Test: inline `#[cfg(test)]` in `vault.rs` and `note_id.rs`

**Interfaces:**
- Consumes: `atomic_write` from Task 5; `NoteMetadata` from Task 2 (`domain`).
- Produces:
  - `pub struct NoteId(String)` (newtype around a validated id) with `impl NoteId { pub fn parse(raw: &str) -> Result<Self, NoteIdError>; pub fn as_str(&self) -> &str; }`
  - `pub enum NoteIdError { Empty, ContainsPathSeparator, ContainsParentRef, NotRelative, MustBeLowercase { found: char }, InvalidCharacter { found: char }, LeadingOrTrailingDot, ReservedName { name: String }, TooLong { max: usize, actual: usize } }` — `parse` rejects empty strings, any `/` or `\`, any `..`, and any absolute path. **Corrected by A1 final-review fix C2:** the original text here allowed `[a-zA-Z0-9_-]+` (mixed case), and the first implementation enforced no charset at all; both let `note-a` and `NOTE-A` alias the same record file on case-insensitive filesystems (Windows, default macOS), which was reproduced as silent data loss. The canonical form is now lowercase ASCII only, `[a-z0-9._-]+`, validated never normalized (uppercase is rejected with `MustBeLowercase`, not folded), with no leading/trailing `.`, no `..`, no Windows reserved device name (`con`, `nul`, `com1`, … with or without extension), at most 128 bytes. The same `NoteId` value is the lock-map key, the record-path source, and (via `HashSet<NoteId>` and validated relation targets) the link-lookup identity in `propose_and_commit`. This is the only constructor for `NoteId`; every `storage` function that currently would take `&str` for a note id instead takes `&NoteId`, so a path-traversal or symlink-escape attempt is rejected before any `Path` is built.
  - `pub struct Version(pub u64)` (newtype, `Copy, Clone, PartialEq, Eq`)
  - `pub struct VaultRecord { pub content: String, pub version: Version }` — the single unit serialized to disk. On-disk representation is one file per note (`<root>/<note_id>.record`) containing a small fixed header (`version` as a decimal line) followed by a `\n---\n` separator and the raw content; this whole buffer is built in memory as one `Vec<u8>` and handed to `atomic_write` as one call, so content and version are replaced by a single rename — there is no second file and no window where one can be read without the other.
  - `pub enum StorageError { Conflict { note_id: String, expected: Version, actual: Version }, NotFound { note_id: String }, PathEscapesVaultRoot { note_id: String }, Io(std::io::Error) }` (final-review fix C1 removed the original `Atomic(AtomicWriteError)` variant: `AtomicWriteError` is crate-private and is folded into `Io` at the crate boundary; final-review fix C2 removed the never-again-constructed `InvalidNoteId(NoteIdError)` variant, because `propose_and_commit` now validates every note id up front and reports it as `ProposeError::InvalidNoteId { field, value, error }`, before storage is reached)
  - `impl Vault { pub fn new(root: PathBuf) -> Self; pub fn current_version(&self, note_id: &NoteId) -> Result<Version, StorageError>; pub fn read_note(&self, note_id: &NoteId) -> Result<VaultRecord, StorageError>; fn write_record_unchecked(&self, note_id: &NoteId, content: &str, expected_version: Version, operation_id: Uuid) -> Result<Version, StorageError>; }`
  - **`write_record_unchecked` is a `pub(crate)` method, never plain `pub`** — it is only ever called from within `storage`: by this task's own unit tests (same module) and, starting in Task 9, by `propose_and_commit` in a sibling file, `crates/storage/src/propose.rs`. `pub(crate)` is required rather than a bare private `fn` precisely because of that sibling-module call: Rust's bare (module-private) visibility does not extend to sibling modules within the same crate, only to the defining module and its descendants, so Task 9's `propose_and_commit` could not otherwise compile against it. `pub(crate)` still makes the primitive fully unreachable from any other crate, including `policy` — this is what makes the "no bypass" guarantee a compiler fact rather than a convention; it is a statement about crate boundaries, not intra-crate module boundaries. `Vault` as built in this task exposes no public write function at all — `pub fn write_note`/`propose_and_commit` is added in Task 9, once `storage` can call `policy::Gate::evaluate` to gate it. Until Task 9 lands, this task's own tests reach the primitive directly because they live inside the same crate.
  - `Vault`'s internal state: `root: PathBuf`, `locks: Mutex<HashMap<String, Arc<Mutex<()>>>>` — document on the struct itself, in a doc comment, that this mutex scope is **per-process, per-`Vault`-instance only**: it serializes concurrent callers that share this `Vault` object (e.g., multiple threads in the same running process), and makes no claim about two separate processes, two separate `Vault::new` calls on the same root, or a crash mid-write — those guarantees require the A2 journal and are explicitly not provided here.
  - `operation_id: Uuid` is accepted and currently unused beyond being threaded through the signature — one-line doc comment stating A1 does not implement cross-restart idempotency, so the parameter exists only so A2 can wire it to journal-backed idempotency without changing this signature.

- [ ] **Step 1: Write failing tests for `NoteId::parse` rejecting path traversal and accepting valid ids**

```rust
#[test]
fn valid_note_id_is_accepted() {
    assert!(NoteId::parse("bug-2026-10-23-fastapi-cors").is_ok());
}

#[test]
fn empty_note_id_is_rejected() {
    assert!(matches!(NoteId::parse(""), Err(NoteIdError::Empty)));
}

#[test]
fn note_id_with_parent_dir_component_is_rejected() {
    assert!(matches!(NoteId::parse("../../etc/passwd"), Err(NoteIdError::ContainsParentRef)));
}

#[test]
fn note_id_with_path_separator_is_rejected() {
    assert!(matches!(NoteId::parse("sub/dir-note"), Err(NoteIdError::ContainsPathSeparator)));
    assert!(matches!(NoteId::parse("sub\\dir-note"), Err(NoteIdError::ContainsPathSeparator)));
}

#[test]
fn absolute_path_as_note_id_is_rejected() {
    assert!(matches!(NoteId::parse("/etc/passwd"), Err(NoteIdError::NotRelative)));
}
```

- [ ] **Step 2: Run tests to verify they fail, implement `NoteId::parse` in `note_id.rs`, run again to verify pass**

Run: `cargo test -p storage note_id` (FAIL, then implement, then PASS). Implementation: check empty first, then scan for `/`/`\`, then scan path `Component`s (via `Path::new(raw).components()`) for `Component::ParentDir` and `Component::RootDir`/prefix, rejecting each with its specific variant.

- [ ] **Step 3: Write failing test for symlink-escape rejection**

```rust
#[test]
fn write_target_resolving_outside_vault_root_via_symlink_is_rejected() {
    let root = unique_test_dir();
    let outside = unique_test_dir(); // a sibling dir, not under root
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&outside, root.join("escape")).unwrap();
    let vault = Vault::new(root.clone());
    // "escape" itself is a syntactically valid NoteId; the vault must still refuse
    // to resolve a record path through it once canonicalization shows it leaves root.
    let note_id = NoteId::parse("escape").unwrap();
    let err = vault.write_record_unchecked(&note_id, "x", Version(0), Uuid::new_v4()).unwrap_err();
    assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }));
}
```

- [ ] **Step 4: Run test to verify it fails, implement the canonicalization check in `Vault`, run again to verify pass**

Run: `cargo test -p storage symlink` (FAIL, then implement, then PASS). Implementation: before any write, build the candidate path `root.join(format!("{}.record", note_id.as_str()))`; canonicalize the **parent directory** of that candidate (the file itself may not exist yet on first write) and confirm the canonicalized parent equals the canonicalized `root` — if not, or if canonicalization fails, return `PathEscapesVaultRoot`.

> **Correction (third whole-branch review, Finding I-C).** This step and the Step 3 test above check the wrong path. Every read and write opens `root/<id>.record`, so a symlink at that exact path is what must be checked. The candidate's parent is always `root` itself, and the Step 3 test plants its link at `root/escape`, which no I/O touches, so it cannot detect an escape. The implemented check canonicalizes the full `.record` path whenever an entry exists there, failing closed if it cannot be resolved, and canonicalizes the parent otherwise. The Step 3 test was replaced; see `eval/results/A1/evidence.md`, Section 9.1. This changes which path is checked, not when: the check-then-use TOCTOU window (Continuity [c12ah]) remains.

- [ ] **Step 5: Write failing test for basic write-then-read as a single atomic record**

```rust
#[test]
fn write_then_read_returns_same_content_and_version_one() {
    let vault = Vault::new(unique_test_dir());
    let id = NoteId::parse("note-a").unwrap();
    let v = vault.write_record_unchecked(&id, "content v1", Version(0), Uuid::new_v4()).unwrap();
    assert_eq!(v, Version(1));
    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.content, "content v1");
    assert_eq!(record.version, Version(1));
}

#[test]
fn record_file_never_shows_mismatched_content_and_version() {
    // After N writes, the on-disk record's embedded version always matches
    // the version current_version() reports — there is exactly one file,
    // so this is a structural guarantee, not a race to test for.
    let vault = Vault::new(unique_test_dir());
    let id = NoteId::parse("note-a").unwrap();
    vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
    vault.write_record_unchecked(&id, "v2", Version(1), Uuid::new_v4()).unwrap();
    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.version, Version(2));
    assert_eq!(record.content, "v2");
}
```

- [ ] **Step 6: Run tests to verify they fail, implement `write_record_unchecked` (first-write case) and `read_note`/`current_version` parsing the single-file format, run again to verify pass**

Run: `cargo test -p storage write_then_read` (FAIL, then implement, then PASS). New note: `expected_version` must be `Version(0)`; if the record file already exists, `Version(0)` as expected is itself a `Conflict`. Serialize `VaultRecord` into one buffer (version line + separator + content) and pass it to `atomic_write` in one call — never two.

- [ ] **Step 7: Write failing test for explicit sequential version conflict and cross-note version non-collision**

```rust
#[test]
fn stale_expected_version_is_rejected_with_explicit_conflict() {
    let vault = Vault::new(unique_test_dir());
    let id = NoteId::parse("note-a").unwrap();
    vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
    let err = vault.write_record_unchecked(&id, "v2 attempt", Version(0), Uuid::new_v4()).unwrap_err();
    match err {
        StorageError::Conflict { note_id, expected, actual } => {
            assert_eq!(note_id, "note-a");
            assert_eq!(expected, Version(0));
            assert_eq!(actual, Version(1));
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.content, "v1");
    assert_eq!(record.version, Version(1));
}

#[test]
fn version_numbers_do_not_collide_across_different_notes() {
    let vault = Vault::new(unique_test_dir());
    let a = NoteId::parse("note-a").unwrap();
    let b = NoteId::parse("note-b").unwrap();
    vault.write_record_unchecked(&a, "a-content", Version(0), Uuid::new_v4()).unwrap();
    let v = vault.write_record_unchecked(&b, "b-content", Version(0), Uuid::new_v4()).unwrap();
    assert_eq!(v, Version(1));
}
```

- [ ] **Step 8: Run tests to verify they fail, implement the version-check-before-write logic, run again to verify pass**

Run: `cargo test -p storage conflict` (FAIL, then implement, then PASS). Implementation: acquire the per-note-id mutex (inserting a fresh `Arc<Mutex<()>>` into `locks` if absent, itself under the short-lived `locks` map mutex), then inside that per-note lock: read current version, compare to `expected_version`, return `Conflict` if mismatched, else build the single-buffer record and call `atomic_write` once, release lock.

- [ ] **Step 9: Write failing tests for true concurrent writers within one process (explicitly scoped to this `Vault` instance) and for retry-after-conflict**

```rust
#[test]
fn concurrent_writers_sharing_one_vault_instance_on_same_version_only_one_succeeds() {
    // Scope of this guarantee: both writers share `vault` (one process, one Vault
    // instance, two threads) — this is exactly what the in-memory mutex can promise.
    let vault = Arc::new(Vault::new(unique_test_dir()));
    let id = NoteId::parse("note-a").unwrap();
    vault.write_record_unchecked(&id, "initial", Version(0), Uuid::new_v4()).unwrap();

    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut handles = vec![];
    for i in 0..2 {
        let vault = Arc::clone(&vault);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            vault.write_record_unchecked(&id, &format!("writer-{i}"), Version(1), Uuid::new_v4())
        }));
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let successes = results.iter().filter(|r| r.is_ok()).count();
    let conflicts = results.iter().filter(|r| matches!(r, Err(StorageError::Conflict { .. }))).count();
    assert_eq!(successes, 1, "exactly one concurrent writer must succeed");
    assert_eq!(conflicts, 1, "the other must get an explicit conflict, not a silent overwrite or panic");

    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.version, Version(2));
}

#[test]
fn losing_writer_can_retry_with_fresh_version_and_succeed() {
    let vault = Vault::new(unique_test_dir());
    let id = NoteId::parse("note-a").unwrap();
    vault.write_record_unchecked(&id, "initial", Version(0), Uuid::new_v4()).unwrap();
    let err = vault.write_record_unchecked(&id, "stale attempt", Version(0), Uuid::new_v4()).unwrap_err();
    assert!(matches!(err, StorageError::Conflict { .. }));
    let current = vault.read_note(&id).unwrap().version;
    let v = vault.write_record_unchecked(&id, "retry with fresh version", current, Uuid::new_v4()).unwrap();
    assert_eq!(v, Version(2));
}
```

(`NoteId` needs `Clone` — add the derive in Step 2 if not already present.)

- [ ] **Step 10: Run tests to verify they fail (if the critical section isn't yet correctly scoped), confirm/fix, verify pass**

Run: `cargo test -p storage concurrent`
Expected: PASS once the per-note mutex genuinely wraps both the version read and the write.

Note: `Vault` as it stands after this task has **no public write function at all** — `write_record_unchecked` is private, reachable only from within `vault.rs`/`storage`. Task 9 adds the single public, gate-backed write entry point (`propose_and_commit`) and, with it, the API-surface test that pins `storage`'s public write surface to exactly that one function.

- [ ] **Step 11: Commit**

```bash
git add crates/storage/src/vault.rs crates/storage/src/note_id.rs crates/storage/src/lib.rs
git commit -m "feat(storage): path-safe, single-record vault writes with explicit conflicts (write primitive is crate-private)"
```

---

## Task 7: Schema migration v1→v2 (domain)

**Files:**
- Create: `crates/domain/src/migration.rs`
- Modify: `crates/domain/src/lib.rs`
- Test: inline `#[cfg(test)]` in `migration.rs`

**Interfaces:**
- Consumes: `NoteMetadata` from Task 2, but operates on a **test-only fixture type**, not on the canonical schema — see decision below.
- Produces:
  - `pub enum MigrationOutcome<T> { Reversible(T), Lossy { migrated: T, lost_fields: Vec<String> } }` (generic, so it is reusable by a real future migration without this task inventing one)
  - `pub enum MigrationError { UnsupportedVersion { from: u32, to: u32 } }`
  - `pub trait Migration { type Input; type Output; fn migrate(&self, input: Self::Input) -> Result<MigrationOutcome<Self::Output>, MigrationError>; }` — a small trait so the idempotency/reversibility *mechanism* is implemented once, generically, and proven with a throwaway fixture, rather than by adding a fake field to `NoteMetadata`.
  - **Decision, replacing the previous approach**: the spec asks A1 to prove the migration *mechanism* (idempotent, reversible-or-declared-lossy, example pairs) works, but schema v2 does not exist yet — the previous plan invented a `migrated_note_id_hint` field on the canonical `NoteMetadata` solely to exercise this, which would ship a fictitious field into the real schema. Instead: define the mechanism generically via the `Migration` trait, and in this task's **test module only** (`#[cfg(test)]`, not exported, not part of `domain`'s public API) declare two small fixture structs `FixtureV1 { id: String, label: String }` and `FixtureV2 { id: String, label: String, note: Option<String> }`, plus a test-local `struct ExampleMigration;` implementing `Migration` for them. This proves the harness without touching `NoteMetadata`/`Classification`/etc. at all. When a real v2 schema is designed (future work, not A1), it implements `Migration` the same way `ExampleMigration` does, with real fields.

- [ ] **Step 1: Write failing tests for the trait mechanism using the test-local fixture, covering migration-pair correctness, idempotency, and reversibility**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct FixtureV1 { id: String, label: String }
    #[derive(Clone, Debug, PartialEq)]
    struct FixtureV2 { id: String, label: String, note: Option<String> }

    struct ExampleMigration;
    impl Migration for ExampleMigration {
        type Input = FixtureV1;
        type Output = FixtureV2;
        fn migrate(&self, input: FixtureV1) -> Result<MigrationOutcome<FixtureV2>, MigrationError> {
            Ok(MigrationOutcome::Reversible(FixtureV2 { id: input.id, label: input.label, note: None }))
        }
    }

    #[test]
    fn example_pair_v1_migrates_to_expected_v2() {
        let v1 = FixtureV1 { id: "x".into(), label: "hello".into() };
        let expected_v2 = FixtureV2 { id: "x".into(), label: "hello".into(), note: None };
        let outcome = ExampleMigration.migrate(v1).unwrap();
        let migrated = match outcome { MigrationOutcome::Reversible(v) => v, _ => panic!("expected reversible") };
        assert_eq!(migrated, expected_v2);
    }

    #[test]
    fn migration_applied_twice_to_the_same_input_yields_the_same_output() {
        let v1 = FixtureV1 { id: "x".into(), label: "hello".into() };
        let once = ExampleMigration.migrate(v1.clone()).unwrap();
        let twice = ExampleMigration.migrate(v1).unwrap();
        let unwrap_reversible = |o: MigrationOutcome<FixtureV2>| match o {
            MigrationOutcome::Reversible(v) => v, _ => panic!("expected reversible"),
        };
        assert_eq!(unwrap_reversible(once), unwrap_reversible(twice));
    }

    #[test]
    fn dropping_the_added_field_reverses_v2_back_to_v1() {
        let v1 = FixtureV1 { id: "x".into(), label: "hello".into() };
        let outcome = ExampleMigration.migrate(v1.clone()).unwrap();
        let v2 = match outcome { MigrationOutcome::Reversible(v) => v, _ => panic!("expected reversible") };
        let reversed = FixtureV1 { id: v2.id, label: v2.label }; // dropping `note` is the declared-reversible path
        assert_eq!(reversed, v1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p domain migration`
Expected: FAIL — `Migration` trait and `MigrationOutcome` not defined.

- [ ] **Step 3: Implement `MigrationOutcome<T>`, `MigrationError`, `Migration` trait in `migration.rs`** (production code is only the generic trait and enum — the fixture/`ExampleMigration` live entirely inside the `#[cfg(test)]` module from Step 1, not in the file's public items)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p domain migration`
Expected: PASS, all 3 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/domain/src/migration.rs crates/domain/src/lib.rs
git commit -m "feat(domain): generic migration mechanism (idempotent, reversible), proven on a test fixture"
```

---

## Task 8: Policy gate (policy)

**Files:**
- Create: `crates/policy/src/gate.rs`
- Create: `crates/policy/src/secrets.rs`
- Modify: `crates/policy/src/lib.rs`
- Test: inline `#[cfg(test)]` in `gate.rs` and `secrets.rs`

**Interfaces:**
- Consumes: `NoteMetadata`, `Classification`, `ValidationError` (Task 2); `LinkState`, `LinkError` (Task 3).
- Produces:
  - `pub enum GateDecision { Pass, Reject(GateRejection) }` — `Pass` carries no payload; no witness token is needed. `Gate::evaluate`'s caller in this workspace is `storage::propose_and_commit` (Task 9), which can call it directly because `storage` depends on `policy` (amended from the spec's original table — see Architecture). Since `policy` depends only on `domain` and has no dependency on `storage`, this is a one-directional edge with no cycle.
  - `pub struct SecretException { pub pattern_id: String, pub matched_text_hash: String, pub reason: String, pub approved_by: String, pub note_id: String }` — **narrowed per the review correction**: an exception is scoped to the exact matched occurrence, not to "this pattern anywhere in this note." `matched_text_hash` is a SHA-256 hex digest (or equivalent) of the specific matched substring the exception covers; `scan` only treats a finding as excepted when `pattern_id`, `note_id`, *and* the hash of that finding's matched text all match a registered exception. This stops one approved false positive from silently whitelisting every future occurrence of the same pattern in the same note (e.g., a real leaked key added later would still be caught even though an earlier doc-placeholder was excepted).
  - `pub struct SecretPolicy { version: u32, exceptions: Vec<SecretException> }` with `impl SecretPolicy { pub fn new(version: u32) -> Self; pub fn with_exception(self, exc: SecretException) -> Self; pub fn scan(&self, content: &str, note_id: &str) -> Vec<SecretFinding>; }`
  - `pub struct SecretFinding { pub pattern_id: String, pub matched_text_hash: String, pub excepted: bool }`
  - `pub enum GateRejection { Schema(ValidationError), Link(LinkError), SecretClassification, SensitiveRequiresEncryptedStore { note_id: String }, SecretContentDetected { note_id: String, pattern_id: String } }`
  - `pub struct Gate { secret_policy: SecretPolicy }` with `impl Gate { pub fn new(secret_policy: SecretPolicy) -> Self; pub fn evaluate(&self, note: &NoteMetadata, content: &str, link_state_result: Result<LinkState, LinkError>) -> GateDecision; }`
- Note: `secrets.rs` uses simple high-confidence regex patterns (e.g., AWS key pattern `AKIA[0-9A-Z]{16}`, a generic `-----BEGIN PRIVATE KEY-----` marker) — this is a minimal, versioned pattern set sufficient for A1's test bullets; it is explicitly not a comprehensive secret-scanner (that's part of the repository-level CI tool from Task 1, a separate concern from this in-process gate check).

- [ ] **Step 1: Write failing tests covering: structurally valid note without relations is not rejected for that reason; `sensitive` classification rejected with the A3 pointer; `secret` classification always rejected; real secret detected and rejected; fictitious example not flagged; false positive with a registered exception passes**

```rust
#[test]
fn note_with_no_relations_passes_gate_on_that_basis_alone() {
    let note = valid_note_with(Classification::Internal, /* unlinked */);
    let gate = Gate::new(SecretPolicy::new(1));
    let decision = gate.evaluate(&note, "plain content", Ok(LinkState::Unlinked));
    assert!(matches!(decision, GateDecision::Pass));
}

#[test]
fn sensitive_classification_is_rejected_pending_encrypted_store() {
    let note = valid_note_with(Classification::Sensitive, /* unlinked */);
    let gate = Gate::new(SecretPolicy::new(1));
    let decision = gate.evaluate(&note, "content", Ok(LinkState::Unlinked));
    assert!(matches!(decision, GateDecision::Reject(GateRejection::SensitiveRequiresEncryptedStore { .. })));
}

#[test]
fn real_secret_pattern_is_rejected() {
    let note = valid_note_with(Classification::Internal, /* unlinked */);
    let gate = Gate::new(SecretPolicy::new(1));
    let content = "aws_key = AKIAABCDEFGHIJKLMNOP";
    let decision = gate.evaluate(&note, content, Ok(LinkState::Unlinked));
    assert!(matches!(decision, GateDecision::Reject(GateRejection::SecretContentDetected { .. })));
}

#[test]
fn fictitious_example_secret_in_prose_without_matching_pattern_is_not_flagged() {
    let note = valid_note_with(Classification::Internal, /* unlinked */);
    let gate = Gate::new(SecretPolicy::new(1));
    let content = "Example: never commit a key like AKIAEXAMPLE1234567 (fake, 19 chars, fails the 16-char pattern)";
    let decision = gate.evaluate(&note, content, Ok(LinkState::Unlinked));
    assert!(matches!(decision, GateDecision::Pass));
}

#[test]
fn false_positive_with_registered_exception_for_that_exact_occurrence_passes() {
    let note = valid_note_with(Classification::Internal, /* unlinked */);
    let content = "aws_key = AKIAABCDEFGHIJKLMNOP";
    let matched_text_hash = sha256_hex("AKIAABCDEFGHIJKLMNOP"); // helper using the same hash fn as scan()
    let exception = SecretException {
        pattern_id: "aws-access-key".into(),
        matched_text_hash,
        reason: "documentation placeholder, verified non-functional".into(),
        approved_by: "francesco".into(),
        note_id: note.id.clone(),
    };
    let policy = SecretPolicy::new(1).with_exception(exception);
    let gate = Gate::new(policy);
    let decision = gate.evaluate(&note, content, Ok(LinkState::Unlinked));
    assert!(matches!(decision, GateDecision::Pass));
}

#[test]
fn exception_for_one_occurrence_does_not_cover_a_different_matched_value() {
    let note = valid_note_with(Classification::Internal, /* unlinked */);
    let excepted_occurrence_hash = sha256_hex("AKIAABCDEFGHIJKLMNOP");
    let exception = SecretException {
        pattern_id: "aws-access-key".into(),
        matched_text_hash: excepted_occurrence_hash,
        reason: "documentation placeholder, verified non-functional".into(),
        approved_by: "francesco".into(),
        note_id: note.id.clone(),
    };
    let policy = SecretPolicy::new(1).with_exception(exception);
    let gate = Gate::new(policy);
    // a different AWS-shaped key, same pattern_id, same note — must still be caught
    let content = "aws_key = AKIAZZZZZZZZZZZZZZZZ";
    let decision = gate.evaluate(&note, content, Ok(LinkState::Unlinked));
    assert!(matches!(decision, GateDecision::Reject(GateRejection::SecretContentDetected { .. })));
}

#[test]
fn secret_classification_is_rejected_before_content_is_even_scanned() {
    let note = valid_note_with(Classification::Internal, /* placeholder, overridden below */);
    // construct with classification literally "secret" — per Task 2, parse_frontmatter already
    // converts this to ValidationError::SecretClassificationForbidden before a NoteMetadata exists,
    // so this test instead verifies Gate::evaluate surfaces GateRejection::SecretClassification
    // when fed a ValidationError::SecretClassificationForbidden via the schema path:
    let decision = Gate::new(SecretPolicy::new(1))
        .evaluate_schema_error(ValidationError::SecretClassificationForbidden);
    assert!(matches!(decision, GateDecision::Reject(GateRejection::SecretClassification)));
}
```

(The last test implies a small additional method `evaluate_schema_error` on `Gate` for the case where `parse_frontmatter` already failed before a `NoteMetadata` exists — add this to the Interfaces block: `pub fn evaluate_schema_error(&self, err: ValidationError) -> GateDecision`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p policy gate`
Expected: FAIL — types not defined.

- [ ] **Step 3: Implement `SecretPolicy::scan`, `SecretFinding`, `Gate::evaluate`, `Gate::evaluate_schema_error` in `secrets.rs`/`gate.rs`**

`scan` runs each versioned regex pattern against `content`; for each match, compute `matched_text_hash` (SHA-256 hex of the matched substring — add a `sha2` crate dependency to `policy`, MIT/Apache-2.0 licensed, allowed per the SPDX list) and check `exceptions` for an entry whose `pattern_id`, `note_id`, and `matched_text_hash` all match that specific finding, marking `excepted: true` only then. `evaluate` short-circuits in this order: schema error → link error → classification (`Sensitive`/`Secret`) → secret content scan (ignoring only findings marked `excepted: true`) → `Pass`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p policy gate`
Expected: PASS, all 7 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/policy/src/gate.rs crates/policy/src/secrets.rs crates/policy/src/lib.rs
git commit -m "feat(policy): gate composing schema/link/classification/secret checks"
```

---

## Task 9: Gate-backed public write entry point, end-to-end tests, and A1 evidence

**Files:**
- Create: `crates/storage/src/propose.rs` — this is the new, real deliverable this task adds: the *only* public write path out of `storage`, and therefore out of the whole workspace. It lives in `storage` (not `policy`) because `storage` is the crate with visibility into both `domain`/`policy` (for the gate) and its own private `write_record_unchecked`.
- Modify: `crates/storage/src/lib.rs` (re-export `propose_and_commit` and `ProposeError`; stop re-exporting any write function other than this one)
- Create: `crates/storage/tests/integration.rs`
- Create: `eval/results/A1/evidence.md`

**Interfaces:**
- Consumes: `policy::Gate::evaluate`/`evaluate_schema_error`, `policy::GateDecision`, `policy::GateRejection` (Task 8); `domain::parse_frontmatter`, `domain::ValidationError` (Task 2); `domain::classify_links`, `domain::LinkError`, `domain::LinkState` (Task 3); `Vault::write_record_unchecked` (Task 6, private — reachable because `propose.rs` lives inside `storage`).
- Produces: `pub fn propose_and_commit(vault: &Vault, note_text: &str, expected_version: Version, operation_id: Uuid, known_ids: &HashSet<NoteId>, pending_ids: &HashSet<String>, gate: &policy::Gate) -> Result<CommitOutcome, ProposeError>` in `storage/src/propose.rs`, with `pub struct CommitOutcome { pub version: Version, pub link_state: domain::LinkState }` (third-review Minor: the return type was `Result<Version, ProposeError>`, which dropped the computed link state, so a pending reference was accepted but not flagged as spec C6 requires) — parses the frontmatter of `note_text`, validates the note id and every relation target *not* declared in `pending_ids` as a canonical `NoteId`, classifies links (via Task 3's `classify_links`, whose `pending_ids: &HashSet<String>` contract above it passes through unchanged), runs the gate over the whole `note_text`, and only on `GateDecision::Pass` calls `vault.write_record_unchecked(...)` with the whole `note_text`. `pub enum ProposeError { Schema(domain::ValidationError), InvalidNoteId { field: String, value: String, error: NoteIdError }, Link(domain::LinkError), Rejected(policy::GateRejection), Storage(StorageError) }`. (Final-review fix C2 changed `known_ids` from `HashSet<String>` to `HashSet<NoteId>` and added `InvalidNoteId`, so link lookups use the same canonical identity as the vault. C2 also changed `pending_ids` to `HashSet<NoteId>`, which the second whole-branch review found broke spec C6: a pending reference may be a URL, a DOI or a code symbol, none of which is a valid `NoteId`. Fix I-A reverted `pending_ids` to opaque `HashSet<String>`: a target present verbatim in `pending_ids` is accepted as pending without `NoteId` parsing. Fix I-B renamed `yaml` to `note_text` and made it accept either bare YAML frontmatter, as before, or a full note: a `---` line, the frontmatter, a `---` line, then an opaque body. The body is stored byte-for-byte in the same record and is scanned by the gate's secret scanner, but it is never parsed.) This is, from this task onward, the **only** `pub fn` in `storage` whose body can reach a filesystem write — `Vault::new`, `current_version`, and `read_note` remain `pub` as read/setup operations, but no second `pub` write function exists anywhere in the crate.

- [ ] **Step 1: Write a failing API-surface test fixing `storage`'s public write surface to exactly one function**

```rust
#[test]
fn storage_exposes_exactly_one_public_write_function() {
    // The "no bypass" guarantee for A1 is: write_record_unchecked is a private
    // method, unreachable from outside this crate, and propose_and_commit is the
    // only public function whose body can reach it. This test pins that shape so
    // a future change adding a second public write path fails here and forces a
    // conscious review, rather than landing silently.
    let vault_source = include_str!("vault.rs");
    let propose_source = include_str!("propose.rs");
    let pub_write_fns_in_vault: Vec<&str> = vault_source.lines()
        .filter(|l| l.trim_start().starts_with("pub fn") && l.contains("write"))
        .collect();
    assert!(pub_write_fns_in_vault.is_empty(), "vault.rs must expose no public write function, found: {pub_write_fns_in_vault:?}");
    let pub_write_fns_in_propose: Vec<&str> = propose_source.lines()
        .filter(|l| l.trim_start().starts_with("pub fn"))
        .collect();
    assert_eq!(pub_write_fns_in_propose.len(), 1, "propose.rs must expose exactly one public function, found: {pub_write_fns_in_propose:?}");
    assert!(pub_write_fns_in_propose[0].contains("propose_and_commit"));
}
```

- [ ] **Step 2: Run test to verify it fails (propose.rs doesn't exist yet), write failing tests for `propose_and_commit`'s acceptance and rejection paths**

```rust
#[test]
fn valid_note_passes_gate_and_is_committed_through_the_controlled_entry_point() {
    let vault = Vault::new(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = /* valid frontmatter from Task 2's fixture, id "note-e2e" */;
    let v = propose_and_commit(&vault, yaml, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate).unwrap();
    assert_eq!(v, Version(1));
}

#[test]
fn secret_classified_note_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created() {
    let vault = Vault::new(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = /* frontmatter with classification: secret, id "note-secret" */;
    let err = propose_and_commit(&vault, yaml, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate).unwrap_err();
    assert!(matches!(err, ProposeError::Rejected(policy::GateRejection::SecretClassification)));
    // positive proof the write never happened, not just that we didn't call write ourselves:
    let id = NoteId::parse("note-secret").unwrap();
    assert!(matches!(vault.read_note(&id).unwrap_err(), StorageError::NotFound { .. }));
}

#[test]
fn note_with_broken_link_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created() {
    let vault = Vault::new(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = /* valid frontmatter, id "note-broken-link", relations.supports: ["missing-id"] */;
    let err = propose_and_commit(&vault, yaml, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate).unwrap_err();
    assert!(matches!(err, ProposeError::Link(domain::LinkError::BrokenLink { .. })));
    let id = NoteId::parse("note-broken-link").unwrap();
    assert!(matches!(vault.read_note(&id).unwrap_err(), StorageError::NotFound { .. }));
}

#[test]
fn version_conflict_surfaces_end_to_end_through_propose_and_commit() {
    let vault = Vault::new(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = /* valid frontmatter, id "note-e2e-conflict" */;
    propose_and_commit(&vault, yaml, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate).unwrap();
    let err = propose_and_commit(&vault, yaml, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate).unwrap_err();
    assert!(matches!(err, ProposeError::Storage(StorageError::Conflict { .. })));
}
```

- [ ] **Step 3: Implement `propose_and_commit` and `ProposeError` in `storage/src/propose.rs`, then run all of Steps 1–2's tests to verify they pass**

Run: `cargo test -p storage propose` and `cargo test -p storage storage_exposes_exactly_one_public_write_function` (both FAIL before this step, both PASS after). Implementation: parse → classify links → `gate.evaluate` → match `GateDecision::Pass` to proceed, else map `GateRejection`/`ValidationError`/`LinkError` to the matching `ProposeError` variant and return before calling `write_record_unchecked` at all; on `Pass`, call `vault.write_record_unchecked(...)` and map any `StorageError` through.

- [ ] **Step 4: Run full workspace tests to confirm no wiring regressions**

Run: `cargo test --workspace`
Expected: all green.

- [ ] **Step 5: Run full verification suite**

Run:
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo deny check
cargo install cargo-mutants --locked
cargo mutants --package domain --package storage --package policy
```
Expected: build/test/clippy/deny all pass; `cargo-mutants` produces a mutation report — every surviving mutant must be examined; for each, either add a test that kills it or write a one-line justification in the evidence file for why it's acceptable (e.g., a mutant in an unreachable `unreachable!()` arm).

- [ ] **Step 6: Write `eval/results/A1/evidence.md`**

Record: git commit hash (`git rev-parse HEAD`), date, exact commands run, raw pass/fail output summary, mutation testing survivor list with disposition (killed by new test / justified), and the explicit acceptance-criteria checklist from the plan's header mapped to pass/fail. Include a line stating A1's boundary: "No SQLite journal, no git commit, no crash recovery, no encryption, no backup/restore in this evidence — those are verified under A2/A3. Cross-process/cross-restart write safety is not provided — only in-process concurrency via a shared `Vault` instance is guaranteed. `propose_and_commit` is the only `pub` write function in `storage`; `write_record_unchecked` is private and unreachable from outside the crate, enforced by the compiler, not by convention. The crate dependency table was amended during A1 implementation: `storage` now depends on `domain` and `policy` (was `domain` only in the spec's original table), so `storage` can call the gate before writing — `policy` still depends only on `domain`, so no cycle exists."

- [ ] **Step 7: Commit**

```bash
git add crates/storage/src/propose.rs crates/storage/src/lib.rs crates/storage/Cargo.toml crates/storage/tests/integration.rs eval/results/A1/evidence.md
git commit -m "feat(storage): add propose_and_commit as the sole gate-backed public write path, plus A1 evidence"
```

---

## Self-Review Notes (completed during authoring)

1. **Spec coverage**: Task 2 covers frontmatter validation test bullet; Task 3 covers broken/pending/unlinked link bullets (C6); Task 8 covers the 4 secret-policy test bullets (real/fictitious/false-positive-exception/derived-note — note: "segreti in note derivate" is satisfied because the gate scans `content` regardless of note type, so a `summary`/`session` note with embedded secret content hits the same `scan` path; no special-casing needed, confirmed by not adding one); Task 6 covers the concurrent-write-conflict, path-traversal, and symlink-escape bullets; Task 7 covers the migration bullet. Crash-injection, cancellation, restore, and the C1 matrix are explicitly out of scope (A2/A3) per the confirmed decision — not silently dropped, but named in Global Constraints and in the evidence file.
2. **Step scan**: each test step includes concrete assertions; each implementation step gives exact signatures and leaves only the method body to the implementer, with algorithmic guidance only where the signature doesn't determine behavior (e.g., mixed pending/resolved link precedence, migration field choice).
3. **Type consistency**: `Version`, `StorageError::Conflict`, `GateDecision`, `GateRejection`, `ValidationError`, `LinkState`, `LinkError`, `NoteId` are defined once (Tasks 2/3/6/8) and referenced identically in Task 9 — checked for name/shape consistency across tasks.
4. **Review Focus**: the six items listed above are each assigned to an owning task's test steps (unknown field type, self-reference link, invalid classification enum, retry-after-conflict, cross-note version collision, path-traversal/symlink escape).
5. **Proportion**: this plan is longer than the spec's A1-relevant excerpt because it is a Rust interface specification (exact signatures across 3 crates) rather than prose; code blocks are test bodies with concrete assertions (unambiguous) and signatures (not full implementations) — implementation bodies are deliberately left to the executor except where an algorithm choice needed pinning (atomic write ordering, link-state precedence, single-record serialization format).
6. **Dependency-direction check (added after the review correction)**: an earlier draft of this fix tried a cross-crate "gate passed" compile-time token while keeping the spec's original table (`policy → storage`, `storage → domain` only) — that is unsatisfiable in Rust's visibility model (no crate can grant construction rights to one specific dependent without a dependency edge toward it), and reversing the edge to `storage → policy` while `policy → storage` still existed would have created a cycle. The resolved design drops `policy → storage` entirely (`policy` depends only on `domain`) and adds `storage → policy` (amended from the spec's original table, documented in the Architecture section and Task 1), so `storage` can call `policy::Gate::evaluate` directly from inside its own crate-private `propose_and_commit`. This makes the "no bypass" guarantee a genuine compiler fact (the raw write primitive is a private method, not merely undocumented) rather than the weaker "structural/testable" fallback considered mid-revision.

## Outstanding technical debt to carry into A2 (do not close silently)

**SEC-001 — Filesystem symlink-race and TOCTOU protection.** Status: DEFERRED, not resolved. A1's symlink-escape check (`crates/storage/src/vault.rs`, now `ensure_resolves_inside_root` after the review-3 fix for finding I-C, which corrected an earlier version that checked the wrong path entirely) verifies the actual `.record` I/O path resolves inside the vault root — but this remains a check-then-act pattern, not a closed TOCTOU window: the filesystem can change between the check and the actual read/write/rename. A2's planning must explicitly reassess this before A2 is declared complete, and it must be reassessed again before Fase D (untrusted code execution) exposes the vault to a materially different threat model. Full detail, required intervention (handle-based opens with the handle itself verified after opening — not another path-level check, which is the pattern that already failed to generalize three times in a row across SEC-001/002/003 below — Windows+Linux-specific protections, adversarial concurrent-modification tests, verifying recovery/backup/restore respect the same boundary), and closure criteria are tracked in `docs/technical-debt.md` under `SEC-001`, and in Continuity as decision `[d5b9]` (related: constraint `[c12ah]`, the original Task 6 TOCTOU finding; constraint `[c15ii]`, the I-C fix that narrowed what gets checked without closing the window; constraint `[c17e8]`, the SEC-003 hard-link finding that disproved SEC-001/002's original "reject symlinks" fix proposal). This item is explicitly out of A1's scope — do not fold it into A1 to "finish early," and do not mark it resolved without the adversarial tests and independent review the debt entry specifies.

**SEC-002 — Intra-vault record aliasing via symlink.** Status: DEFERRED, not resolved. Found during the fix-I-C scoped re-review (third review round): a symlink at one note's `.record` path pointing at ANOTHER note's own `.record` file, both inside the vault root, passes the "resolves inside root" check (it technically does) without being an escape — causing cross-note content/version reads (including the aliased note inheriting the target's version number) rather than data corruption. The fourth whole-branch review found the originally-proposed fix ("reject any symlink at a record path") does not also cover SEC-003 (hard links) — both need the same handle-verification mechanism as SEC-001, not a path-level check. Tracked in `docs/technical-debt.md` under `SEC-002`, Continuity constraint `[c16c6]`.

**SEC-003 — Intra- and extra-vault aliasing via hard link.** Status: DEFERRED, not resolved. Found during the fourth whole-branch review: a hard link at a `.record` path (to a file outside the vault, or to another note's own `.record` file inside it) produces the identical aliasing effect as SEC-001/SEC-002's symlink cases, but canonicalization cannot detect it — a hard link is a second directory entry to the same inode, not a redirect, so the path genuinely is a regular file genuinely inside the root. Must close alongside SEC-001/SEC-002 in A2 using the same open-then-verify-the-handle mechanism (link count must be 1, in addition to "regular file" and "path inside root"). Tracked in `docs/technical-debt.md` under `SEC-003`, Continuity constraint `[c17e8]`.
