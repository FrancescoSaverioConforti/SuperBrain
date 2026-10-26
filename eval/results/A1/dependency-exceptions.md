# A1 dependency exceptions

Recorded per the project spec: every exception is justified, time-bound, and recorded here.

## Exception 1: RUSTSEC-2024-0449

- **Advisory:** RUSTSEC-2024-0449, `unsafe-libyaml` is unmaintained. The upstream repository was archived by its owner on 2024-03-25. Last release 0.2.11 (March 2024).
- **Affected crate:** `unsafe-libyaml` 0.2.11.
- **Dependency chain:** `superbrain` crates → `domain` → `serde_yaml` 0.9.34 → `unsafe-libyaml` 0.2.11. The `serde_yaml` dependency is declared in `crates/domain/Cargo.toml`.
- **Justification:** `serde_yaml` 0.9 is pinned by the implementation plan. `unsafe-libyaml` is a transitive dependency with no fixed upstream release as of 2026-10-26. Replacing `serde_yaml` is out of scope for A1 and is not done here.
- **Scope of risk:** YAML parsing of the project's own data files. No network input is parsed.
- **Date granted:** 2026-10-26.
- **Re-review by:** 2027-04-26 (6 months). At that date, check for an available fix (a newer `serde_yaml` release or a replacement in the plan). If none exists, renew this exception with a new date or escalate.
- **Recorded in:** `deny.toml`, `[advisories] ignore` entry with the same reason and dates.
