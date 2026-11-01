//! C4 calendar ranges in Europe/Rome. Kept in index while search is the only
//! consumer; move to domain if C2 needs the same contract. No clock or vault I/O.
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, TimeZone};
use chrono_tz::Europe::Rome;
use chrono_tz::Tz;

/// Typed input from the caller's parser. Omitted-year expressions require a
/// vault-derived list of years containing that month; None means unknown,
/// Some(empty) means the caller checked and found no matching content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeExpression {
    Month {
        month: u32,
        year: Option<i32>,
        content_years: Option<Vec<i32>>,
    },
    LastMonth,
    MonthsAgo(u32),
    AroundDate(NaiveDate),
    LastWeek,
    /// An identified event must supply its already-resolved, bounded interval.
    /// No unbounded "before" interval is guessed from a missing event.
    BeforeEvent {
        resolved_range: Option<(NaiveDate, NaiveDate)>,
    },
    Explicit {
        start: NaiveDate,
        end: NaiveDate,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClarificationReason {
    MonthYear,
    UnidentifiedEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeResolutionError {
    NeedsClarification(ClarificationReason),
    InvalidDate,
    InvalidRange,
    OutOfRange,
    NonUniqueLocalMidnight,
}

impl std::fmt::Display for TimeResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NeedsClarification(_) => "time expression needs clarification",
            Self::InvalidDate => "invalid calendar date",
            Self::InvalidRange => "range must have start before end",
            Self::OutOfRange => "calendar arithmetic out of range",
            Self::NonUniqueLocalMidnight => "local midnight is missing or ambiguous",
        })
    }
}
impl std::error::Error for TimeResolutionError {}

/// Resolve local calendar boundaries to a half-open interval in Europe/Rome.
/// MonthsAgo is a whole calendar month, never a rolling count of days.
/// The caller must display this convention, zone and selected temporal field
/// in the search response (Task 6); this pure resolver does not render a UI.
pub fn resolve_time_expression(
    expr: &TimeExpression,
    reference_date: NaiveDate,
) -> Result<(DateTime<Tz>, DateTime<Tz>), TimeResolutionError> {
    use TimeResolutionError::*;
    let (start, end) = match expr {
        TimeExpression::Month {
            month,
            year,
            content_years,
        } => {
            if !(1..=12).contains(month) {
                return Err(InvalidDate);
            }
            let year = match year {
                Some(year) => *year,
                None => {
                    let years = content_years
                        .as_ref()
                        .ok_or(NeedsClarification(ClarificationReason::MonthYear))?;
                    if years.iter().any(|year| Some(year) != years.first()) {
                        return Err(NeedsClarification(ClarificationReason::MonthYear));
                    }
                    reference_date
                        .year()
                        .checked_sub(i32::from(*month > reference_date.month()))
                        .ok_or(OutOfRange)?
                }
            };
            let start = NaiveDate::from_ymd_opt(year, *month, 1).ok_or(OutOfRange)?;
            month_range(start)?
        }
        TimeExpression::LastMonth => months_ago(reference_date, 1)?,
        TimeExpression::MonthsAgo(n) => months_ago(reference_date, *n)?,
        TimeExpression::AroundDate(date) => (
            date.checked_sub_days(Days::new(7)).ok_or(OutOfRange)?,
            // C4 includes the seventh following day: midnight of day +8 is
            // the exclusive upper bound (Aug 23 -> Aug 16 .. Aug 31).
            date.checked_add_days(Days::new(8)).ok_or(OutOfRange)?,
        ),
        TimeExpression::LastWeek => {
            let end = reference_date
                .checked_sub_days(Days::new(u64::from(
                    reference_date.weekday().num_days_from_monday(),
                )))
                .ok_or(OutOfRange)?;
            (end.checked_sub_days(Days::new(7)).ok_or(OutOfRange)?, end)
        }
        TimeExpression::BeforeEvent { resolved_range } => {
            resolved_range.ok_or(NeedsClarification(ClarificationReason::UnidentifiedEvent))?
        }
        TimeExpression::Explicit { start, end } => (*start, *end),
    };
    if start >= end {
        return Err(InvalidRange);
    }
    // Resolve each wall-clock boundary independently using the IANA rules.
    // Never add fixed 24-hour durations to timezone-aware timestamps.
    Ok((rome_midnight(start)?, rome_midnight(end)?))
}

fn month_range(start: NaiveDate) -> Result<(NaiveDate, NaiveDate), TimeResolutionError> {
    let end = start
        .checked_add_months(Months::new(1))
        .ok_or(TimeResolutionError::OutOfRange)?;
    Ok((start, end))
}

fn months_ago(reference: NaiveDate, n: u32) -> Result<(NaiveDate, NaiveDate), TimeResolutionError> {
    let first = reference
        .with_day(1)
        .ok_or(TimeResolutionError::OutOfRange)?;
    let start = first
        .checked_sub_months(Months::new(n))
        .ok_or(TimeResolutionError::OutOfRange)?;
    month_range(start)
}

fn rome_midnight(date: NaiveDate) -> Result<DateTime<Tz>, TimeResolutionError> {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .ok_or(TimeResolutionError::InvalidDate)?;
    Rome.from_local_datetime(&midnight)
        .single()
        .ok_or(TimeResolutionError::NonUniqueLocalMidnight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Offset, TimeZone};
    use chrono_tz::Europe::Rome;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn reference() -> NaiveDate {
        date("2026-10-26")
    }
    fn assert_range(expr: TimeExpression, reference: NaiveDate, start: &str, end: &str) {
        let (a, b) = resolve_time_expression(&expr, reference).unwrap();
        assert_eq!(a.date_naive(), date(start));
        assert_eq!(b.date_naive(), date(end));
        assert_eq!(a.time(), chrono::NaiveTime::MIN);
        assert_eq!(b.time(), chrono::NaiveTime::MIN);
        assert_eq!(a.timezone(), Rome);
        assert_eq!(b.timezone(), Rome);
    }
    #[test]
    fn ad_agosto_uses_most_recent_non_future_august() {
        assert_range(
            TimeExpression::Month {
                month: 8,
                year: None,
                content_years: Some(vec![2026]),
            },
            reference(),
            "2026-08-01",
            "2026-09-01",
        );
    }
    #[test]
    fn il_mese_scorso_is_the_previous_calendar_month() {
        assert_range(
            TimeExpression::LastMonth,
            reference(),
            "2026-09-01",
            "2026-10-01",
        );
    }
    #[test]
    fn due_mesi_fa_is_a_whole_month_not_sixty_days() {
        assert_range(
            TimeExpression::MonthsAgo(2),
            reference(),
            "2026-08-01",
            "2026-09-01",
        );
    }
    #[test]
    fn intorno_al_23_agosto_includes_seven_days_either_side() {
        assert_range(
            TimeExpression::AroundDate(date("2026-08-23")),
            reference(),
            "2026-08-16",
            "2026-08-31",
        );
    }
    #[test]
    fn la_settimana_scorsa_is_previous_iso_week() {
        assert_range(
            TimeExpression::LastWeek,
            reference(),
            "2026-10-19",
            "2026-10-26",
        );
    }
    #[test]
    fn end_of_month_reference_does_not_clamp_the_target_month() {
        assert_range(
            TimeExpression::LastMonth,
            date("2026-01-31"),
            "2025-12-01",
            "2026-01-01",
        );
    }
    #[test]
    fn omitted_year_with_multiple_content_years_needs_clarification() {
        assert_eq!(
            resolve_time_expression(
                &TimeExpression::Month {
                    month: 8,
                    year: None,
                    content_years: Some(vec![2025, 2026])
                },
                reference()
            ),
            Err(TimeResolutionError::NeedsClarification(
                ClarificationReason::MonthYear
            ))
        );
    }
    #[test]
    fn omitted_year_with_unknown_coverage_needs_clarification() {
        assert_eq!(
            resolve_time_expression(
                &TimeExpression::Month {
                    month: 8,
                    year: None,
                    content_years: None
                },
                reference()
            ),
            Err(TimeResolutionError::NeedsClarification(
                ClarificationReason::MonthYear
            ))
        );
    }
    #[test]
    fn year_rollover_two_months_before_january_is_november() {
        assert_range(
            TimeExpression::MonthsAgo(2),
            date("2026-01-15"),
            "2025-11-01",
            "2025-12-01",
        );
    }
    #[test]
    fn leap_february_has_twenty_nine_calendar_days() {
        let (start, end) =
            resolve_time_expression(&TimeExpression::LastMonth, date("2028-03-31")).unwrap();
        assert_eq!(start.date_naive(), date("2028-02-01"));
        assert_eq!(end.date_naive(), date("2028-03-01"));
        assert_eq!(end.signed_duration_since(start).num_days(), 29);
    }
    #[test]
    fn rome_autumn_dst_day_is_twenty_five_hours() {
        let (start, end) = resolve_time_expression(
            &TimeExpression::Explicit {
                start: date("2026-10-25"),
                end: date("2026-10-26"),
            },
            reference(),
        )
        .unwrap();
        assert_eq!(start.offset().fix().local_minus_utc(), 7200);
        assert_eq!(end.offset().fix().local_minus_utc(), 3600);
        assert_eq!(end.signed_duration_since(start), Duration::hours(25));
        assert_eq!(start.to_rfc3339(), "2026-10-25T00:00:00+02:00");
        assert_eq!(end.to_rfc3339(), "2026-10-26T00:00:00+01:00");
    }
    #[test]
    fn rome_spring_dst_day_is_twenty_three_hours() {
        let (start, end) = resolve_time_expression(
            &TimeExpression::Explicit {
                start: date("2026-03-29"),
                end: date("2026-03-30"),
            },
            reference(),
        )
        .unwrap();
        assert_eq!(end.signed_duration_since(start), Duration::hours(23));
        assert_eq!(start.offset().fix().local_minus_utc(), 3600);
        assert_eq!(end.offset().fix().local_minus_utc(), 7200);
    }
    #[test]
    fn iso_week_crosses_dst_using_calendar_days() {
        let (start, end) = resolve_time_expression(&TimeExpression::LastWeek, reference()).unwrap();
        assert_eq!(end.signed_duration_since(start), Duration::hours(169));
    }
    #[test]
    fn prima_della_migrazione_without_event_needs_clarification() {
        assert_eq!(
            resolve_time_expression(
                &TimeExpression::BeforeEvent {
                    resolved_range: None
                },
                reference()
            ),
            Err(TimeResolutionError::NeedsClarification(
                ClarificationReason::UnidentifiedEvent
            ))
        );
    }
    #[test]
    fn identified_event_uses_the_callers_resolved_range() {
        assert_range(
            TimeExpression::BeforeEvent {
                resolved_range: Some((date("2026-01-01"), date("2026-06-03"))),
            },
            reference(),
            "2026-01-01",
            "2026-06-03",
        );
    }
    #[test]
    fn interval_includes_start_and_excludes_end() {
        let (start, end) =
            resolve_time_expression(&TimeExpression::LastMonth, reference()).unwrap();
        let contains = |t| start <= t && t < end;
        assert!(contains(start));
        assert!(!contains(start - Duration::nanoseconds(1)));
        assert!(contains(end - Duration::nanoseconds(1)));
        assert!(!contains(end));
        assert_eq!(
            start,
            Rome.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).single().unwrap()
        );
    }
    #[test]
    fn future_named_month_rolls_back_a_year() {
        assert_range(
            TimeExpression::Month {
                month: 12,
                year: None,
                content_years: Some(vec![]),
            },
            reference(),
            "2025-12-01",
            "2026-01-01",
        );
    }
    #[test]
    fn same_named_month_uses_current_year_and_duplicate_years_are_not_ambiguous() {
        assert_range(
            TimeExpression::Month {
                month: 10,
                year: None,
                content_years: Some(vec![2026, 2026]),
            },
            reference(),
            "2026-10-01",
            "2026-11-01",
        );
    }
    #[test]
    fn explicit_year_disambiguates_multiple_content_years() {
        assert_range(
            TimeExpression::Month {
                month: 8,
                year: Some(2024),
                content_years: Some(vec![2024, 2025, 2026]),
            },
            reference(),
            "2024-08-01",
            "2024-09-01",
        );
    }
    #[test]
    fn last_week_from_sunday_still_uses_previous_iso_week() {
        assert_range(
            TimeExpression::LastWeek,
            date("2027-01-03"),
            "2026-12-21",
            "2026-12-28",
        );
    }
    #[test]
    fn zero_months_ago_is_current_calendar_month() {
        assert_range(
            TimeExpression::MonthsAgo(0),
            reference(),
            "2026-10-01",
            "2026-11-01",
        );
    }
    #[test]
    fn invalid_month_is_a_hard_error() {
        for month in [0, 13, u32::MAX] {
            assert_eq!(
                resolve_time_expression(
                    &TimeExpression::Month {
                        month,
                        year: Some(2026),
                        content_years: None
                    },
                    reference()
                ),
                Err(TimeResolutionError::InvalidDate)
            );
        }
    }
    #[test]
    fn empty_or_reversed_explicit_range_is_a_hard_error() {
        for end in ["2026-10-26", "2026-10-25"] {
            assert_eq!(
                resolve_time_expression(
                    &TimeExpression::Explicit {
                        start: reference(),
                        end: date(end)
                    },
                    reference()
                ),
                Err(TimeResolutionError::InvalidRange)
            );
        }
    }
    #[test]
    fn extreme_calendar_arithmetic_returns_errors_without_panicking() {
        for (expr, reference) in [
            (TimeExpression::MonthsAgo(u32::MAX), reference()),
            (TimeExpression::LastMonth, NaiveDate::MIN),
            (TimeExpression::LastWeek, NaiveDate::MIN),
            (TimeExpression::AroundDate(NaiveDate::MIN), reference()),
            (TimeExpression::AroundDate(NaiveDate::MAX), reference()),
            (TimeExpression::MonthsAgo(0), NaiveDate::MAX),
        ] {
            assert_eq!(
                resolve_time_expression(&expr, reference),
                Err(TimeResolutionError::OutOfRange)
            );
        }
    }

    #[test]
    fn time_errors_have_nonempty_sanitized_diagnostics() {
        for error in [
            TimeResolutionError::NeedsClarification(ClarificationReason::MonthYear),
            TimeResolutionError::InvalidDate,
            TimeResolutionError::InvalidRange,
            TimeResolutionError::OutOfRange,
            TimeResolutionError::NonUniqueLocalMidnight,
        ] {
            assert!(!error.to_string().trim().is_empty());
        }
    }
}
