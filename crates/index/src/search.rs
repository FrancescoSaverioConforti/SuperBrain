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

/// B3-TUNE-v2 Priority 3 Stage 1, Lever P1: a fixed additive bonus
/// applied to a candidate's BM25 score when `Index::with_identifier_boost`
/// is enabled and its frozen exact-identifier rule fires (see
/// `apply_identifier_boost`). Pinned well above any realistically
/// achievable BM25 score on this corpus: with `N` = 53 indexed notes
/// (`eval/results/B3/calibration-20261114T123333Z-e613edd.json`'s
/// `corpus_manifest_note_count`), the IDF ceiling for a term unique to
/// one document is `ln((53 - 1 + 0.5) / (1 + 0.5) + 1) ≈ 4.07`, and the
/// BM25 term-frequency factor saturates toward `k1 + 1 = 2.2` as `tf`
/// grows — so a single term's score ceiling is in the ~9 range, and a
/// multi-term query summing several such terms could in principle reach
/// into the tens, but not into the hundreds on a corpus this small.
/// `100.0` is chosen as a round constant comfortably above that
/// ceiling, so the boost reliably promotes a matching candidate to the
/// top of the ranking rather than merely nudging it.
const IDENTIFIER_BOOST: f64 = 100.0;

/// Frozen contract (B4.0,
/// `docs/superpowers/specs/2026-11-15-b4-production-contract-freeze.md`).
/// Selects which retrieval pipeline `Index::search` uses to generate and
/// score candidates. Filtering, privacy/redaction, and the final top-k
/// truncation are IDENTICAL across every variant — only candidate
/// generation and scoring differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RetrievalMode {
    /// Default. Byte-identical to every `Index::search` call site's
    /// behavior before this enum existed — B1 trigram+BM25 only. Zero
    /// behavior change for any caller that does not explicitly opt in.
    #[default]
    Lexical,
    /// Opt-in. `v2 MiniLM dense-only`, per
    /// `eval/results/B3-TUNE-v2/frozen-config-v2.json`. Falls back to
    /// `Lexical` behavior, with an explicit logged warning, if the dense
    /// model/index is structurally confirmed unavailable — never a hard
    /// `SearchError` for that reason alone.
    DenseV2,
}

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
    /// B3-TUNE-v2 Priority 3 Stage 1 Lever P1 (ablation use only — see
    /// `with_identifier_boost`'s own doc comment). Defaults to `false`
    /// in `new()`; every existing call site is unaffected.
    identifier_boost: bool,
    /// B3-TUNE-v2 Priority 3 Stage 1 Lever P2 (ablation use only — see
    /// `with_status_tiebreak`'s own doc comment). Defaults to `false`
    /// in `new()`; every existing call site is unaffected.
    status_tiebreak: bool,
    /// B4.0 frozen contract (see `RetrievalMode`'s own doc comment).
    /// Defaults to `RetrievalMode::Lexical` in `new()`; every existing
    /// call site is unaffected.
    retrieval_mode: RetrievalMode,
}

/// B3-TUNE-v2 Priority 3 Stage 1, Lever P2's frozen status-tier mapping
/// (`docs/superpowers/specs/2026-11-14-b3-tune-v2-priority3-stage1-design.md`),
/// confirmed against the real 6-variant `domain::NoteStatus` (which has
/// no derived `Ord`): lower number sorts first (higher relevance tier).
/// Tier 1: Verified, Reviewed. Tier 2: TestPassed. Tier 3: Draft.
/// Tier 4: Superseded, Contradicted. Same-tier members (e.g. Verified
/// vs. Reviewed) are NOT ordered by this function — they fall through
/// to the existing note-id tie-break.
fn status_tier(status: NoteStatus) -> u8 {
    match status {
        NoteStatus::Verified | NoteStatus::Reviewed => 1,
        NoteStatus::TestPassed => 2,
        NoteStatus::Draft => 3,
        NoteStatus::Superseded | NoteStatus::Contradicted => 4,
    }
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
            identifier_boost: false,
            status_tiebreak: false,
            retrieval_mode: RetrievalMode::default(),
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

    /// Additive builder for B3-TUNE-v2 Priority 3 Stage 1 Lever P1
    /// (ablation use only): when enabled, a candidate whose
    /// `source:`-extracted `#<IDENTIFIER>` fragment exactly matches one
    /// of the query's whitespace tokens (case-insensitive, never a
    /// substring/prefix match) gets `IDENTIFIER_BOOST` added to its
    /// final BM25 score before sorting. Never changes candidate
    /// membership — only reorders candidates already present. Default
    /// `false`; every existing call site of `new()` is unaffected.
    pub fn with_identifier_boost(mut self) -> Self {
        self.identifier_boost = true;
        self
    }

    /// Additive builder for B3-TUNE-v2 Priority 3 Stage 1 Lever P2
    /// (ablation use only): when enabled, two candidates whose final
    /// BM25 scores are bitwise-equal `f64` values (never within an
    /// epsilon — a real, however small, score difference always wins)
    /// are additionally ordered by `status_tier`, before the existing
    /// note-id tie-break. Never changes candidate membership. Default
    /// `false`; every existing call site of `new()` is unaffected.
    pub fn with_status_tiebreak(mut self) -> Self {
        self.status_tiebreak = true;
        self
    }

    /// Additive builder (B4.0 frozen contract,
    /// `docs/superpowers/specs/2026-11-15-b4-production-contract-freeze.md`).
    /// Selects `retrieval_mode` for this `Index` instance. Default (never
    /// called): `RetrievalMode::Lexical`, identical to every pre-B4 call
    /// site.
    pub fn with_retrieval_mode(mut self, mode: RetrievalMode) -> Self {
        self.retrieval_mode = mode;
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
        // Parallel to `fresh_results`, same push order: `document.status`
        // (Copy) captured before `document` is consumed by the push below,
        // for Lever P2's status tie-break (search_impl's sort reads this
        // by index position, never by note id, so order must stay in sync).
        let mut fresh_result_statuses: Vec<NoteStatus> = Vec::new();
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
            fresh_result_statuses.push(document.status);
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
        let query_tokens: HashSet<String> =
            query.split_whitespace().map(|t| t.to_lowercase()).collect();
        for result in &mut fresh_results {
            let base_score = scores.get(&result.note_id).copied().unwrap_or(0.0);
            let boost = if self.identifier_boost && identifier_boost_applies(&result.provenance.source, &query_tokens) {
                IDENTIFIER_BOOST
            } else {
                0.0
            };
            result.score = base_score + boost;
        }
        // Paired with `fresh_result_statuses` (same push order, Section
        // above) so Lever P2's status tie-break can compare each pair's
        // status without widening `SearchResult`'s own public fields.
        let mut paired: Vec<(SearchResult, NoteStatus)> =
            fresh_results.into_iter().zip(fresh_result_statuses).collect();
        let status_tiebreak = self.status_tiebreak;
        paired.sort_by(|(a, a_status), (b, b_status)| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| {
                    if status_tiebreak && a.score == b.score {
                        status_tier(*a_status).cmp(&status_tier(*b_status))
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| a.note_id.as_str().cmp(b.note_id.as_str()))
        });
        let mut fresh_results: Vec<SearchResult> = paired.into_iter().map(|(r, _)| r).collect();
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

/// B3-TUNE-v2 Priority 3 Stage 1 Lever P1: true only if `source` has a
/// `#<IDENTIFIER>` fragment that exactly equals (case-insensitive) one
/// of `query_tokens`. `source: None` (withheld for `Sensitive` results,
/// see `search_impl`'s redaction at lines above) always returns `false`
/// — the boost never applies to a candidate whose source is withheld.
fn identifier_boost_applies(source: &Option<String>, query_tokens: &HashSet<String>) -> bool {
    let Some(source) = source else {
        return false;
    };
    let Some(identifier) = extract_structural_identifier(source) else {
        return false;
    };
    query_tokens.contains(&identifier.to_lowercase())
}

/// Reads `source`'s trailing `#<IDENTIFIER>` fragment (e.g.
/// `docs/technical-debt.md#SEC-001` -> `Some("SEC-001")`), read up to
/// the first non-identifier character. Returns `None` if `source` has
/// no `#` fragment, or the fragment is empty. Same extraction rule as
/// `b3_tune_v2_lever_c_alias_table.rs`'s `extract_structural_candidates`
/// (duplicated here, not shared, since that function lives in an
/// example binary this crate cannot depend on and additionally parses
/// a whole note's frontmatter rather than a bare `source` string).
fn extract_structural_identifier(source: &str) -> Option<String> {
    let fragment_start = source.find('#')?;
    let fragment = &source[fragment_start + 1..];
    let identifier: String = fragment
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!identifier.is_empty()).then_some(identifier)
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
