//! Opt-in aggregate timings. No ids, query text, excerpts or content retained.
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone)]
pub struct SearchProfile {
    pub candidate_generation: Duration,
    pub authoritative_reads: Duration,
    /// Parsing, current Document/query/filters and fresh scoring-corpus insert.
    pub revalidation: Duration,
    /// Excerpts/provenance, fresh BM25 ranking, sorting and response assembly.
    pub response_building: Duration,
    pub candidates: usize,
    pub reads: usize,
    pub eligible: usize,
}

pub(crate) struct PhaseTimer<'a> {
    sink: Option<&'a mut Duration>,
    start: Option<Instant>,
}
impl<'a> PhaseTimer<'a> {
    pub(crate) fn new(sink: Option<&'a mut Duration>) -> Self {
        let start = sink.as_ref().map(|_| Instant::now());
        Self { sink, start }
    }
}
impl Drop for PhaseTimer<'_> {
    fn drop(&mut self) {
        if let (Some(sink), Some(start)) = (&mut self.sink, self.start) {
            **sink += start.elapsed();
        }
    }
}
