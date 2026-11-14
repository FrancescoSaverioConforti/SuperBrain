---
id: code-time-range-resolve-time-expression
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: crates/index/src/time_range.rs:resolve_time_expression
created: 2026-10-26
updated: 2026-10-26
event_period: 2026-10-26
relations: {}
---
repository: superbrain
commit: e92dc781bcd32fcca5445295221f36aea2fe791e
path: crates/index/src/time_range.rs
symbol: resolve_time_expression

pub fn resolve_time_expression(expr: &TimeExpression, reference_date: NaiveDate) -> Result<(DateTime<Tz>, DateTime<Tz>), TimeResolutionError>

Resolves local calendar boundaries to a half-open interval in Europe/Rome. MonthsAgo always means a whole calendar month, never a rolling count of days.
