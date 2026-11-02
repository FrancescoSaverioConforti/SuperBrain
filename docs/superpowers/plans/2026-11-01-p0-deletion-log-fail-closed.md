# P0 DeletionLog fail-closed implementation plan

Status: authorized architecture; implementation pending RED tests and independent state-machine review.

This plan implements only DL-02/DL-03 from the approved security audit at
`14ae361`. It does not change `docs/Spec_mvp.md`, claim project-wide security,
or address PiiStore traversal, AEAD/AAD, backup provenance, performance, or an
independent anti-rollback anchor.

## Security boundary and residual risk

Cooperating SuperBrain processes share one cross-process deletion-log lock.
Every state-changing DeletionLog operation and every consumer decision that
could expose a note must use that lock. A filesystem/host writer that ignores
the lock is outside this P0 guarantee, though removal or replacement observed
during a guarded operation must be rejected. Replaying an older, mutually
authentic DB+reference pair remains possible without an independent monotonic
anchor and remains an open NO-GO risk.

## Persistent states

The database schema has an explicit format/version marker and the authenticated
reference has a versioned envelope. Genesis is represented by both:

- a valid, empty, versioned database; and
- an authenticated reference `{ format_version, generation: 0,
  chain_hash: GENESIS_PREV_CHAIN_HASH }`.

Absence is never genesis. No read/open API creates a directory, database,
schema, reference, lock parent, or marker.

| Observed state | Classification | Allowed operation |
| --- | --- | --- |
| DB absent + reference absent | `Uninitialized` for a proven-new explicit target; otherwise `LegacyNeedsMigration` | `bootstrap_new` only with explicit new-target precondition, or separate attested migration |
| DB absent + reference present | `CorruptOrTampered` | deny |
| DB present + reference absent | `LegacyNeedsMigration` only for recognized legacy-empty DB; otherwise `CorruptOrTampered` | separate attested migration or deny |
| DB empty/versioned + authenticated generation 0 reference | `Genesis` | open/read/write |
| DB non-empty/versioned + matching authenticated reference | `Current` or recoverable `Pending` | open; reconcile only the already-defined safe pending transitions |
| Current unversioned DB with generation >=1 + matching authenticated legacy reference + existing lock artifact | `LegacyVerifiedCurrent` | open/read/write without rewriting; all full-chain/reference/lock checks still apply |
| Valid current unversioned DB/reference generation >=1 but lock artifact absent | `LegacyNeedsMigration` | explicit administrative coordination activation only; `open_existing` does not create it |
| Unknown schema/format/version | `Incompatible` | deny without rewriting |
| Broken chain, malformed/tampered reference, generation/hash disagreement, replaced/removed identity | `CorruptOrTampered` | deny |
| Old authentic DB+reference pair | indistinguishable from a former valid state | allowed by this P0 but documented as unresolved rollback risk |

`Uninitialized` is returned only by an explicit bootstrap inspection of a
caller-declared new target. `open_existing` must report missing legacy paths as
`LegacyNeedsMigration`; it cannot infer newness. Migration is a distinct,
auditable API/command boundary that requires an explicit attestation token or
typed authorization argument. It never runs from `open_existing`, `PiiStore`,
`Vault`, backup, restore, search, or retrieve and never rewrites automatically.

The current pre-P0 format is not rejected indiscriminately. A non-empty current
DB at generation 1 or later, with a complete valid chain and its matching
authenticated legacy `GenerationRef`, is a `LegacyVerifiedCurrent` state. It is
accepted without rewriting only when its existing lock artifact is also present,
because it contains positive authenticated deletion history and coordination
state. If that lock is absent, a distinct administrative coordination activation
creates only that artifact and records a receipt; normal open still fails.
Missing/empty legacy state has no deletion proof and remains migration-only.
The same distinction applies to backup snapshots: a generation >=1 legacy
snapshot can be checked against a verified live log; absence/empty sentinel is
quarantined. This compatibility mode does not turn an empty legacy state into
genesis and does not add an anti-rollback claim.

### Bootstrap crash state machine

Bootstrap owns three create-new artifacts: the lock file, authenticated
generation-0 reference, and versioned DB. Each carries the same random,
non-secret `store_id` and format version; the reference authenticates the
binding. The order is lock -> reference -> DB, with durable temp-file writes and
atomic promotion at each boundary.

| Bootstrap observation | Transition |
| --- | --- |
| none exist | create lock, authenticated genesis reference, then genesis DB |
| lock only | retry may continue only through explicit `bootstrap_new`; consumers deny |
| lock + valid authenticated genesis reference, DB absent | retry reconstructs only the empty versioned genesis DB with the same `store_id`; consumers deny until complete |
| DB present but reference absent, IDs mismatch, non-genesis reference, rows present, or malformed partial artifact | no bootstrap recovery; `CorruptOrTampered`/administrative quarantine |
| all three match | bootstrap retry returns the original non-sensitive receipt; no rewrite |

This ordering deliberately makes the only automatic explicit-bootstrap retry
derive from an authenticated generation-0 reference. `open_existing` never
performs it. A failure that cannot prove this exact partial bootstrap is not
guessed; an administrator must inspect or quarantine it.

## API and locking model

1. Replace ambiguous `DeletionLog::open` with:
   - `DeletionLog::bootstrap_new(paths)`: the only initializer; implements the
     crash state machine above and returns `BootstrapReceipt { store_id,
     format_version }` without note data.
   - `DeletionLog::open_existing(paths)`: receives one `DeletionLogPaths`
     value containing fixed DB, reference and lock paths; SQLite uses no-create
     flags and validates file type, schema and format without creating anything.
   - an explicitly named administrative migration entry point for an attested
     recognized legacy-empty state; normal consumers cannot call it implicitly.
2. Add typed, non-sensitive errors including `Uninitialized`,
   `LegacyNeedsMigration`, `CorruptOrTampered`, `Incompatible`, lock/IO errors,
   and the existing recovery-specific distinctions where useful.
3. `DeletionLog` is an already-bound handle: consumers cannot supply a different
   reference per lookup/write. Its authenticated reference includes the same
   `store_id` as DB metadata in the P0 format, so DB-A/reference-B is rejected
   before lookup. A verified legacy pair has no `store_id`; it is bound by full
   chain plus head generation/hash only. A copied authentic legacy pair with the
   same head remains part of the explicitly unresolved rollback risk.
4. Introduce a guarded verified lookup. Under the cross-process lock and one
   SQLite snapshot/connection it validates format, the whole chain, the
   authenticated reference, legal pending state, and the requested note.
5. Consumer callbacks/read guards keep the same lock alive through reading the
   Vault record or PII ciphertext. Before returning data, compare the opened DB
   and lock identities/metadata with their paths and re-read/authenticate the
   bound reference; fail if any disappeared, changed, or was replaced. A lookup
   boolean detached from the read is not a valid consumer API.
6. Writers (`record_deletion`, reconcile, restore promotion) use the same lock
   ordering already documented: Vault note lock -> DeletionLog lock -> SQLite
   connection. Do not acquire a Vault lock while holding the DeletionLog lock.
7. Existing pending-deletion recovery and idempotent restore behavior remain;
   genesis generation 0 becomes the reference predecessor for generation 1.

The lock artifact is created only by bootstrap/migration. Every normal open uses
no-create and fails if it is absent, so a consumer cannot silently replace lost
coordination state.

### Administrative legacy migration

The library exposes migration separately from normal handles. Its owner is an
explicit administrative tool/caller, never a data consumer. It accepts
`LegacyNoDeletionAttestation` containing a caller identity/authority label,
timestamp, reason and a random operation ID, none of which contains note data.
The operator is responsible for the truth of “no historic deletion”; the code
can prove only the recognized empty/absent layout.

Preconditions are: exclusive target lock where one exists; no deletion rows;
no authenticated reference claiming generation >0; recognized legacy schema or
fully absent DB/reference; and no unknown/partial files. Migration creates the
same bound generation-0 state as bootstrap and persists an authenticated,
non-sensitive migration receipt in DB metadata and the versioned reference.
The returned receipt repeats operation ID, store ID and format version.

Migration uses the bootstrap ordering and recovery table. Retrying the same
operation ID is idempotent and returns the persisted receipt; a different
operation ID, changed precondition, partial state without a matching
authenticated receipt, or any row/reference history fails closed. This is an
auditable security event, not a claim that the program independently proved the
attestation. No automatic vault/backup rewrite or bulk migration is provided.
For a verified generation >=1 legacy DB/reference pair whose only missing
artifact is the lock, a narrower administrative coordination activation creates
that lock and an audit receipt without changing DB rows or reference bytes.

## Consumer migration

Production call sites to update and pin with source/API tests:

- `PiiStore::get_unless_deleted`: verified guarded PII read.
- `Vault::open_with_pii_store`: `open_existing` plus guarded reconciliation;
  never bootstrap.
- `Vault::read_note`, `list_note_ids`, and write-side `refuse_if_deleted`:
  guarded verified decisions; non-Sensitive notes are covered too.
- recovery `note_is_deleted`: guarded verified lookup while deciding forward
  versus rollback.
- propose/delete: open an existing verified writer; never create.
- backup creation: take a guarded, verified snapshot of chain + authenticated
  reference; remove the old `(empty log, absent reference)` sentinel exception.
- restore: require authenticated genesis/current live state; legacy empty/absent
  backups are quarantined and cannot use the old sentinel, while verified
  generation >=1 legacy snapshots retain read compatibility.
- search/retrieve: end-to-end tests through `Vault::list_note_ids` and
  `Vault::read_note`, including Sensitive and non-Sensitive retired notes.

The application/bootstrap owner must call `bootstrap_new` explicitly when
creating a genuinely new store. `PiiStore::open` remains unable to choose
between bootstrap and existing state.

## TDD sequence and deterministic RED tests

### RED 1: lifecycle and format

- explicit bootstrap creates DB + authenticated generation-0 reference;
- retry of the same bootstrap identity continues or returns its receipt
  idempotently; a different bootstrap identity or an unrelated partial state
  fails without overwrite;
- crash/retry at lock creation, reference promotion and DB promotion follows the
  bootstrap table and never lets a consumer repair state;
- `open_existing` on absent state creates no files/directories;
- absent/altered reference, missing DB, corrupt schema, unknown format/version,
  broken middle link and mismatched generation/hash are distinct fail-closed
  errors without note content;
- recognized legacy absent/empty states require migration; migration requires
  explicit attestation, persists a receipt, is idempotent by operation ID and is
  tested at every crash boundary; no consumer invokes it;
- current non-empty generation >=1 legacy pairs and backup snapshots remain
  usable after complete verification, without on-disk rewriting;
- old authentic pair test documents the unresolved rollback limitation rather
  than asserting rejection.

### RED 2: coordinated lookup and races

Use barriers/hooks, never sleeps:

- remove DB after handle open and before guarded lookup;
- remove/alter reference after handle open;
- replace DB with an empty/new/older log while a handle exists;
- pair DB-A with reference-B: P0 format rejects mismatched `store_id`; legacy
  format rejects chain/head mismatches and documents an identical authentic pair
  as the unresolved rollback case;
- replace/remove/change the bound reference or lock during a guarded read;
- mutate a middle chain row;
- pause consumer after verification, start deletion, prove deletion blocks until
  the read finishes and subsequent reads deny;
- pause deletion after Pending insert and prove all readers deny;
- verify multiple handles/process-cooperative lock behavior and bounded failure.

### RED 3: consumer matrix

For Sensitive and non-Sensitive notes, exercise missing DB, missing/tampered ref,
broken chain, replacement/rollback fixture, and concurrent deletion across:

- PiiStore gated read;
- Vault open/read/list/write;
- recovery roll-forward/rollback and pending deletion;
- propose/delete idempotency;
- backup and restore promotion/reopen, including legacy-backup quarantine;
- index search and API retrieve/search end-to-end.

Every test asserts no plaintext/record/result is returned and no missing state is
silently recreated. Restore/recovery regression fixtures cover all existing
crash points and repeated invocation.

### GREEN implementation order

1. Typed lifecycle errors and versioned generation reference.
2. Create-new bootstrap and no-create open-existing.
3. Full verified lookup under shared lock plus DB identity recheck.
4. Writer/reconcile adaptation preserving pending recovery.
5. PiiStore and Vault consumer conversion.
6. Recovery/propose conversion.
7. Backup/restore quarantine behavior.
8. Search/retrieve end-to-end coverage and documentation of residual findings.

## Verification and checkpoint

- Record RED failures before production changes and GREEN results afterward.
- Run focused package tests, then `cargo test --workspace --locked`.
- Run targeted mutation testing on: missing-file branches, generation-0 checks,
  chain/reference checks, lookup result inversion, identity recheck, and lock
  lifetime. Surviving mutants are failures unless justified in evidence.
- Run `cargo clippy --workspace --locked -- -D warnings` and
  `cargo deny check --disable-fetch`.
- Obtain independent review of this state machine before code and independent
  review of the final implementation/diff after GREEN.
- Update the security audit/evidence with closed DL-02/DL-03 scope and all open
  risks, retaining the global NO-GO verdict.
- Create a focused commit only after tests/reviews. Do not merge, tag, or push.

