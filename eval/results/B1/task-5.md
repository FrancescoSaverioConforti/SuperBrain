# B1 Task 5 - C4 relative date resolution

Date: 2026-10-30. Baseline: f1c2abfee3419096e0c432f2384f8675fd32e8d5,
branch b1-deterministic-search, clean at takeover. Main and the historical
A2 worktree were left untouched. Tasks 0-4 were not reimplemented.

## Behavior and scope

`index::time_range::resolve_time_expression` is a pure typed resolver with an
explicit `NaiveDate` reference. It returns `[start, end)` as `DateTime<Tz>` in
Europe/Rome. Supported inputs: named month (optional year), previous month,
N calendar months ago, seven days around an explicit date, previous ISO week,
a caller-resolved event range, and explicit date boundaries. No wall clock,
storage access, free-text parser, search API or new persistence is introduced.

Unknown month-year coverage and multiple distinct content years require typed
clarification. The caller must supply coverage from the vault; None is unknown,
Some(empty) is checked-empty. An explicit year bypasses ambiguity. Otherwise the
most recent non-future month is selected, as C4 requires. Missing event identity
requires clarification; a known event must arrive with an already bounded range.
No lower bound for an unbounded 'before' query is invented here.

Calendar arithmetic is checked. Each local midnight is independently resolved
against IANA Europe/Rome; missing/ambiguous midnight is rejected. Invalid/reversed
ranges and overflow return typed errors. Around Aug 23 ends Aug 31 exclusive,
matching the canonical C4 example. The plan's January/two-months-ago parenthetical
mentions December incorrectly: the test pins November of the preceding year.
The MVP v6 specification and approved plan were not changed.

Placement in index is intentional while search is the sole consumer; reconsider
moving to domain if C2 reuses it. Task 6 owns field dispatch/filtering and rendering
the temporal field, timezone and potentially surprising calendar convention.

## TDD and validation

Baseline independently run: cargo build --workspace, cargo test --workspace
(430 passed), cargo clippy --workspace -- -D warnings.

RED: 24 new named tests compiled and all 24 failed on the unimplemented resolver.
GREEN: all 24 passed after implementation. Coverage includes every C4 table row,
end of month, omitted-year ambiguity, year rollover, leap February, inclusive and
exclusive boundaries, ISO weeks across year boundaries, invalid input and checked
arithmetic at representational limits.

The autumn DST test asserts 2026-10-25 midnight +02:00 to 2026-10-26 midnight
+01:00 and 25 elapsed hours. The previous ISO week has 169 hours. Spring has
23 hours. These assert real timezone rules rather than assuming 24-hour days.

Final commands:

- cargo build --workspace: passed.
- cargo test --workspace --quiet: 454 passed, 0 failed, 0 ignored.
  Counts: domain 32; index unit 62; index integration 6; policy 14;
  storage unit 201; storage integrations 4+6+14+24; vault-security 86;
  storage doctests 5.
- cargo clippy --workspace -- -D warnings: passed, zero warnings.
- cargo deny check: advisories/bans/licenses/sources all passed. Existing warning
  categories remain (duplicate versions and unused license allowances).
- git diff --check: passed.

Mutation testing and scalability benchmarks remain Task 9 work.

## Dependency audit

Chrono does not include the IANA timezone database; its companion chrono-tz is
necessary to implement the plan's DateTime<Tz> API with real Rome DST. Two direct
crates (rather than a single calendar-only dependency) and five transitives were
added. Versions and checksums are locked in Cargo.lock; no existing package was
upgraded. Chrono defaults are disabled, enabling only std, without clock/now or
host-timezone features. Timezone data is compiled in; runtime stays offline.

| Package | Version | License |
| --- | --- | --- |
| chrono | 0.4.45 | MIT OR Apache-2.0 |
| chrono-tz | 0.10.4 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| phf | 0.12.1 | MIT |
| phf_shared | 0.12.1 | MIT |
| siphasher | 1.0.4 | MIT OR Apache-2.0 |

Upstream license sources: [chrono](https://docs.rs/crate/chrono/0.4.45/source/LICENSE.txt),
[chrono-tz](https://docs.rs/crate/chrono-tz/0.10.4/source/LICENSE),
[num-traits](https://github.com/rust-num/num-traits),
[autocfg](https://github.com/cuviper/autocfg),
[phf/phf_shared](https://github.com/rust-phf/rust-phf),
[siphasher](https://github.com/jedisct1/rust-siphash).
Each downloaded package's Cargo.toml license declaration was checked locally,
as well as cargo-deny's full transitive license check before implementation.
chrono-tz's upstream license identifies tzdb files as public domain.
No allowlist changes or exceptions were added.

## Continuity and remaining security obligations

The exact reviewer-verified Task 4 quote was checked against handoff section 8
and recorded in .continuity/claims/n2io.md on this branch. Status stays open:
postings and indexing log/error surfaces covered; SearchResponse body/source/
relations confidentiality tests remain mandatory for Task 6; embedding, summary,
attachment and cache remain open. SEC-001/002/003 stay deferred. No storage write
path, Indexer/Vault locking contract or Sensitive allowlist was changed.

## Independent review

GO from the independent task5_review agent; no Critical/Important residual
findings. The reviewer independently ran cargo test -p index --locked --quiet
(62 unit + 6 integration passed), cargo clippy --workspace --locked -- -D warnings
(clean), and cargo deny check (all four categories OK). They inspected real Rome
DST arithmetic, month/year/event ambiguity, calendar boundaries, dependency
licenses and the n2io update.

Preliminary findings fixed and rechecked: accidental formatting of four existing
index files removed; manifest BOM/trailing blank removed; missing n2io quotation
actually copied from the verified Task 4 report. No unrelated code changes remain.
Four initial reviewer indexer-test failures were caused by sandbox denial on
TEMP git repositories; rerunning outside the sandbox passed all tests.

Task 5 stops here. Task 6 has not started. No merge, tag or push performed.
