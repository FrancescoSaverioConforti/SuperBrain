//! Read-only search over indexed candidates, always resolved through the vault.
//!
//! Candidates are the union of literal-substring and BM25 term hits, without
//! a preliminary k limit. Each is read through the authoritative gates, parsed
//! again, matched and filtered using its CURRENT classification and metadata.
//! The response never contains a cached index score, source or excerpt.
//!
//! Scores are BM25 over the freshly read, matching, filter-eligible candidates
//! in this request, not over a stale global corpus. Exact substring-only hits
//! can have score 0; MatchKind records why they are included. Comparisons are
//! meaningful within a response, not across project scopes or queries.
//!
//! Date grammar: YYYY-MM-DD or RFC3339 instants; event periods also accept
//! start/end in those forms, with half-open overlap semantics. Malformed date
//! fields cannot pass that field's date filter. Dates resolve to Rome midnight.
//! Excerpts use the whole matching body, or the allowlisted searchable metadata
//! if the body does not match. B1 has no summarization or snippet-size policy.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use domain::{Classification, NoteStatus, NoteType, TrustLevel};
use storage::{NoteId, StorageError, Vault, Version};

use crate::bm25::{tokenize, Bm25Index};
use crate::diagnostics::{PhaseTimer, SearchProfile};
use crate::document::{Document, EventPeriod};
use crate::indexer_impl::RealIndexer;
use crate::time_range::{resolve_time_expression, TimeExpression, TimeResolutionError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProjectScope {
    #[default]
    ActiveOnly,
    Explicit(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeField {
    CreatedAt,
    UpdatedAt,
    EventPeriod,
    Any,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    pub project: ProjectScope,
    pub type_filter: Option<NoteType>,
    pub status_filter: Option<NoteStatus>,
    pub trust_filter: Option<TrustLevel>,
    pub date_filter: Option<(TimeField, TimeExpression)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliedProjectScope {
    ActiveOnly {
        project: String,
    },
    Explicit {
        active_project: String,
        searched_project: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeConvention {
    CalendarMonthNotRollingDays,
    PreviousIsoWeek,
    SevenDaysEitherSide,
    ExplicitRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedTimeFilter {
    pub field: TimeField,
    pub start: DateTime<Tz>,
    pub end: DateTime<Tz>,
    pub timezone: &'static str,
    pub convention: TimeConvention,
    pub reference_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchContext {
    pub project: AppliedProjectScope,
    pub time: Option<AppliedTimeFilter>,
    /// No cross-note snapshot or complete/current index guarantee is made.
    pub completeness: Completeness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    IndexedCandidatesMayBeIncomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub project: String,
    pub trust: TrustLevel,
    pub classification: Classification,
    /// Withheld on the search surface for Sensitive notes, including Debug.
    pub source: Option<String>,
    /// B1 note records do not carry repository/commit provenance. Never guess.
    pub repository_id: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    ExactSubstring,
    RankedTerms,
}

/// C6's gate result is not persisted in B1 records. Nonempty relations cannot
/// honestly be labelled Resolved/Pending from a record alone. Do not expose
/// the raw targets just to explain that uncertainty, especially for Sensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceState {
    Unlinked,
    NotRevalidated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub note_id: NoteId,
    pub version: Version,
    pub excerpt: String,
    pub score: f64,
    pub match_kind: MatchKind,
    pub provenance: Provenance,
    pub references: ReferenceState,
}

/// Structurally nonempty: Matches always contains its first result.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchOutcome {
    NotPresent,
    Matches {
        first: SearchResult,
        rest: Vec<SearchResult>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResponse {
    pub context: SearchContext,
    pub outcome: SearchOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchError {
    InvalidLimit,
    EmptyQuery,
    EmptyProject,
    /// Fail closed on IO, journal, deletion-log or other operational failures.
    /// No raw storage error, note content, id or local path is echoed.
    ReadUnavailable,
    TimeResolution(TimeResolutionError),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidLimit => "search limit must be positive",
            Self::EmptyQuery => "search query must not be empty",
            Self::EmptyProject => "search project must not be empty",
            Self::ReadUnavailable => "search could not verify vault readability",
            Self::TimeResolution(_) => "search time expression could not be resolved",
        })
    }
}
impl std::error::Error for SearchError {}

pub struct Index<'a> {
    vault: &'a Vault,
    indexer: &'a RealIndexer,
    active_project: String,
    reference_date: NaiveDate,
    /// Final-scoring BM25 params, defaulted to the conventional (1.2,
    /// 0.75) in `new()`. Overridable only via `with_bm25_params` — see
    /// its own doc comment; every existing call site of `new()` is
    /// unaffected.
    bm25_k1: f64,
    bm25_b: f64,
}

impl<'a> Index<'a> {
    pub fn new(
        vault: &'a Vault,
        indexer: &'a RealIndexer,
        active_project: &str,
        reference_date: NaiveDate,
    ) -> Self {
        Self {
            vault,
            indexer,
            active_project: active_project.to_owned(),
            reference_date,
            bm25_k1: 1.2,
            bm25_b: 0.75,
        }
    }

    /// Additive builder for custom final-scoring BM25 `k1`/`b` (ablation
    /// use only — the "A-score" variant: this overrides only the
    /// per-query `fresh_scores` index built inside `search`, never
    /// `indexer`'s own persistent candidate-prefilter `Bm25Index`, which
    /// stays at whatever params `indexer` itself was constructed with).
    pub fn with_bm25_params(mut self, k1: f64, b: f64) -> Self {
        self.bm25_k1 = k1;
        self.bm25_b = b;
        self
    }

    pub fn search(
        &self,
        query: &str,
        filters: SearchFilters,
        k: usize,
    ) -> Result<SearchResponse, SearchError> {
        self.search_impl(query, filters, k, None)
    }

    /// Same validated path, with aggregate timings only. Normal search does
    /// not read the profiling clock. Errors retain the normal sanitized type.
    pub fn search_profiled(
        &self,
        query: &str,
        filters: SearchFilters,
        k: usize,
    ) -> Result<(SearchResponse, SearchProfile), SearchError> {
        let mut profile = SearchProfile::default();
        let response = self.search_impl(query, filters, k, Some(&mut profile))?;
        Ok((response, profile))
    }

    fn search_impl(
        &self,
        query: &str,
        filters: SearchFilters,
        k: usize,
        mut profile: Option<&mut SearchProfile>,
    ) -> Result<SearchResponse, SearchError> {
        if k == 0 {
            return Err(SearchError::InvalidLimit);
        }
        let query = query.trim();
        if query.is_empty() {
            return Err(SearchError::EmptyQuery);
        }
        let project = match &filters.project {
            ProjectScope::ActiveOnly => self.active_project.as_str(),
            ProjectScope::Explicit(project) => project.as_str(),
        };
        if project.trim().is_empty() {
            return Err(SearchError::EmptyProject);
        }
        let project_context = match &filters.project {
            ProjectScope::ActiveOnly => AppliedProjectScope::ActiveOnly {
                project: project.to_owned(),
            },
            ProjectScope::Explicit(_) => AppliedProjectScope::Explicit {
                active_project: self.active_project.clone(),
                searched_project: project.to_owned(),
            },
        };
        let time = filters
            .date_filter
            .as_ref()
            .map(|(field, expr)| {
                let (start, end) = resolve_time_expression(expr, self.reference_date)
                    .map_err(SearchError::TimeResolution)?;
                let convention = match expr {
                    TimeExpression::Month { .. }
                    | TimeExpression::LastMonth
                    | TimeExpression::MonthsAgo(_) => TimeConvention::CalendarMonthNotRollingDays,
                    TimeExpression::LastWeek => TimeConvention::PreviousIsoWeek,
                    TimeExpression::AroundDate(_) => TimeConvention::SevenDaysEitherSide,
                    TimeExpression::Explicit { .. } | TimeExpression::BeforeEvent { .. } => {
                        TimeConvention::ExplicitRange
                    }
                };
                Ok(AppliedTimeFilter {
                    field: *field,
                    start,
                    end,
                    timezone: "Europe/Rome",
                    convention,
                    reference_date: self.reference_date,
                })
            })
            .transpose()?;

        let mut fresh_scores = Bm25Index::new_with_params(self.bm25_k1, self.bm25_b);
        let mut fresh_results = Vec::new();
        let candidates = {
            let _timer =
                PhaseTimer::new(profile.as_deref_mut().map(|p| &mut p.candidate_generation));
            self.indexer.candidate_ids(query)
        };
        if let Some(p) = profile.as_deref_mut() {
            p.candidates = candidates.len();
        }
        for id in candidates {
            // Only rejected individual candidates are silently omitted.
            // Operational/gate unavailability must not become Ok(NotPresent).
            let read = {
                if let Some(p) = profile.as_deref_mut() {
                    p.reads += 1;
                }
                let _timer =
                    PhaseTimer::new(profile.as_deref_mut().map(|p| &mut p.authoritative_reads));
                self.vault.read_note(&id)
            };
            let record = match read {
                Ok(record) => record,
                Err(
                    StorageError::Deleted { .. }
                    | StorageError::NoteDeleted { .. }
                    | StorageError::NotFound { .. }
                    | StorageError::Inconsistent { .. },
                ) => continue,
                Err(_) => return Err(SearchError::ReadUnavailable),
            };
            let validation = PhaseTimer::new(profile.as_deref_mut().map(|p| &mut p.revalidation));
            let Ok((metadata, body)) = storage::split_record(record.content.as_bytes()) else {
                continue;
            };
            if metadata.id != id.as_str() {
                continue;
            }
            let sensitive = metadata.classification == Classification::Sensitive;
            let relations = &metadata.relations;
            let references = if relations.supports.is_empty()
                && relations.contradicts.is_empty()
                && relations.derived_from.is_empty()
                && relations.supersedes.is_empty()
                && relations.applies_to.is_empty()
            {
                ReferenceState::Unlinked
            } else {
                ReferenceState::NotRevalidated
            };
            let source = if sensitive {
                None
            } else {
                Some(metadata.source.clone())
            };
            let document = Document::new(metadata, body.as_deref());
            let Some(match_kind) = query_match(&document.text_searchable, query) else {
                continue;
            };
            if document.project != project
                || filters.type_filter.is_some_and(|v| v != document.r#type)
                || filters.status_filter.is_some_and(|v| v != document.status)
                || filters.trust_filter.is_some_and(|v| v != document.trust)
                || time.as_ref().is_some_and(|t| !matches_time(&document, t))
            {
                continue;
            }
            fresh_scores.insert(id.clone(), &document.text_searchable);
            drop(validation);
            if let Some(p) = profile.as_deref_mut() {
                p.eligible += 1;
            }
            let _response_timer =
                PhaseTimer::new(profile.as_deref_mut().map(|p| &mut p.response_building));
            let excerpt = match body {
                Some(body) if !sensitive && query_match(&body, query).is_some() => body,
                _ => document.text_searchable,
            };
            fresh_results.push(SearchResult {
                note_id: id,
                version: record.version,
                excerpt,
                score: 0.0,
                match_kind,
                provenance: Provenance {
                    project: document.project,
                    trust: document.trust,
                    classification: document.classification,
                    source,
                    repository_id: None,
                    commit: None,
                },
                references,
            });
        }
        let _response_timer = PhaseTimer::new(profile.map(|p| &mut p.response_building));
        let scores: HashMap<_, _> = fresh_scores
            .query_ranked(query, usize::MAX)
            .into_iter()
            .collect();
        for result in &mut fresh_results {
            result.score = scores.get(&result.note_id).copied().unwrap_or(0.0);
        }
        fresh_results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.note_id.as_str().cmp(b.note_id.as_str()))
        });
        fresh_results.truncate(k);
        let mut results = fresh_results.into_iter();
        let outcome = match results.next() {
            None => SearchOutcome::NotPresent,
            Some(first) => SearchOutcome::Matches {
                first,
                rest: results.collect(),
            },
        };
        Ok(SearchResponse {
            context: SearchContext {
                project: project_context,
                time,
                completeness: Completeness::IndexedCandidatesMayBeIncomplete,
            },
            outcome,
        })
    }
}

/// Same literal matching as trigram's confirmation, or any BM25 token overlap.
/// The tokenizer is shared with BM25 so re-validation cannot drift from it.
fn query_match(text: &str, query: &str) -> Option<MatchKind> {
    let needle = query.to_lowercase();
    if needle.chars().count() >= 3 && text.to_lowercase().contains(&needle) {
        return Some(MatchKind::ExactSubstring);
    }
    let terms: HashSet<_> = tokenize(text).into_iter().collect();
    tokenize(query)
        .iter()
        .any(|term| terms.contains(term))
        .then_some(MatchKind::RankedTerms)
}

fn matches_time(document: &Document, filter: &AppliedTimeFilter) -> bool {
    let point_matches = |point: &DateTime<Tz>| point >= &filter.start && point < &filter.end;
    let created = || document.created.as_ref().is_some_and(point_matches);
    let updated = || document.updated.as_ref().is_some_and(point_matches);
    let event = || {
        document
            .event_period
            .as_ref()
            .is_some_and(|period| match period {
                EventPeriod::Point(point) => point_matches(point),
                EventPeriod::Interval { start, end } => start < &filter.end && end > &filter.start,
            })
    };
    match filter.field {
        TimeField::CreatedAt => created(),
        TimeField::UpdatedAt => updated(),
        TimeField::EventPeriod => event(),
        TimeField::Any => created() || updated() || event(),
    }
}
