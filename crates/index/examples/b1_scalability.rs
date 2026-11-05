//! Task 9 protocol runner, not runtime search code. Never reads eval labels.
//! Usage: b1_scalability N output-directory [query-limit; default 100] [synthetic-vault-root].
//! Authorized deviation: 100 queries/scale, exactly 40 exact/40 BM25/20 filtered.
//! The original N-dependent full protocol is NOT claimed as executed.
use chrono::NaiveDate;
use index::{
    indexer_impl::{rebuild_index, RealIndexer},
    search::{SearchFilters, SearchOutcome, TimeField},
    time_range::TimeExpression,
    Index,
};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    time::Instant,
};
use storage::{NoteId, StorageError, Vault, Version};
use uuid::Uuid;

const SEED: u64 = 0xB1_2026_1030;
fn draw(state: &mut u64, n: usize) -> usize {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state % n as u64) as usize
}

fn note(i: usize) -> (String, String) {
    let id = format!("bench-note-{i:06}");
    let (kind, body) = match i % 4 {
        0 => ("bug", format!("Request handler request_{i} retries after a transient database timeout. Regression test regression_{i} covers cancellation and verifies the transaction is rolled back.")),
        1 => ("decision", format!("Decision decision_{i} keeps configuration in the project repository. Explicit review is required before enabling a new integration.\n\nAlternatives considered: environment overrides and a separate configuration service. The local file makes deployment reproducible and avoids network availability dependencies. Implementation task task_{i} documents migration and rollback.")),
        2 => ("pattern", format!("Pattern pattern_{i} separates parsing from validation. The parser parser_{i} returns structured input and the validation stage checks identifiers before storage access.")),
        _ => ("research", format!("Experiment experiment_{i} compares query latency across corpus sizes.\n\nMethod: populate committed notes, rebuild the derived index, then measure repeated lookups with an explicit project and time range. Memory use is sampled before and after rebuild.\n\nObservation: candidate fanout can dominate read validation cost. Additional test test_{i} distinguishes rare identifiers from common terms. Followup measurement followup_{i} checks whether result limits reduce work before authoritative reads.\n\nLimit: this observation concerns deterministic retrieval only and does not evaluate generated answers or semantic similarity.")),
    };
    let month = i % 12 + 1;
    let text = format!("---\nid: {id}\nschema_version: 1\ntype: {kind}\nstatus: draft\ntrust: internal\nclassification: internal\nproject: bench\nsource: synthetic:seed-b120261030\ncreated: 2026-{month:02}-15\nupdated: 2026-{month:02}-15\nevent_period: 2026-{month:02}-15\nrelations: {{}}\n---\n{body}");
    (text, body)
}

#[cfg(windows)]
fn memory() -> (usize, usize) {
    // Layout and ABI: Microsoft PROCESS_MEMORY_COUNTERS/GetProcessMemoryInfo.
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        paged_peak: usize,
        paged: usize,
        nonpaged_peak: usize,
        nonpaged: usize,
        pagefile: usize,
        pagefile_peak: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            cb: u32,
        ) -> i32;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: repr(C) matches the documented structure, its size is supplied;
    // process is our valid pseudo-handle, counters is writable for the call.
    assert_ne!(
        unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) },
        0
    );
    (counters.working, counters.peak)
}
#[cfg(not(windows))]
fn memory() -> (usize, usize) {
    let text = fs::read_to_string("/proc/self/status").expect("memory accounting unavailable");
    let field = |name| {
        text.lines()
            .find(|s| s.starts_with(name))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<usize>()
            .unwrap()
            * 1024
    };
    (field("VmRSS:"), field("VmHWM:"))
}

struct Query {
    kind: &'static str,
    text: String,
    filtered: bool,
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let n: usize = args[1].parse().unwrap();
    assert!(n >= 100);
    let out = PathBuf::from(&args[2]);
    fs::create_dir_all(&out).unwrap();
    let wait = args.get(3).is_some_and(|s| s == "wait");
    let mut limit: usize = args
        .get(3)
        .filter(|_| !wait)
        .map(|s| s.parse().unwrap())
        .unwrap_or(100);
    let root = args.get(4).map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("superbrain-b1-scale-{n}-{}", Uuid::new_v4()))
    });
    fs::create_dir_all(&root).unwrap();
    // Setup is not timed. Public sanctioned writes only; a non-git vault keeps
    // durable JournalCommitted content with explicit GitPending outcomes. This
    // isolates the three retrieval metrics from N git commits during setup.
    let vault =
        Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let mut corpus =
        BufWriter::new(File::create(out.join(format!("corpus-{n}.mdstream"))).unwrap());
    let mut vocabulary = Vec::new();
    let empty = HashSet::new();
    let pending = HashSet::new();
    let setup = Instant::now();
    for i in 0..n {
        let (text, body) = note(i);
        vocabulary.extend(body.split_whitespace().map(str::to_owned));
        writeln!(corpus, "{}\n{}", text.len(), text).unwrap();
        let id = NoteId::parse(&format!("bench-note-{i:06}")).unwrap();
        match vault.read_note(&id) {
            Err(StorageError::NotFound { .. }) => {
                let result = storage::propose_and_commit(
                    &vault,
                    &text,
                    Version(0),
                    Uuid::new_v4(),
                    &empty,
                    &pending,
                    &gate,
                    None,
                )
                .unwrap();
                assert!(matches!(result, storage::CommitResult::GitPending { .. }));
            }
            Ok(record) => {
                // Resume only identical deterministic synthetic records,
                // confirmed through the authoritative gates; never raw SQL.
                assert_eq!(record.version, Version(1));
                assert_eq!(record.content, text, "existing synthetic corpus differs");
            }
            Err(error) => panic!("synthetic fixture cannot be authoritatively read: {error}"),
        }
        if (i + 1) % 1000 == 0 {
            eprintln!(
                "{n}: populated/verified {} in {:.1}s",
                i + 1,
                setup.elapsed().as_secs_f64()
            );
        }
    }
    corpus.flush().unwrap();
    assert_eq!(vault.list_note_ids().unwrap().len(), n);
    let mut rng = SEED;
    let mut queries = Vec::new();
    // Authorized fixed-size stratified sample, deterministically selected.
    for _ in 0..20 {
        for _ in 0..2 {
            queries.push(Query {
                kind: "exact",
                text: format!("bench-note-{:06}", draw(&mut rng, n)),
                filtered: false,
            });
        }
        for filtered in [false, false, true] {
            let count = 2 + draw(&mut rng, 3);
            let text = (0..count)
                .map(|_| vocabulary[draw(&mut rng, vocabulary.len())].clone())
                .collect::<Vec<_>>()
                .join(" ");
            queries.push(Query {
                kind: if filtered { "filtered" } else { "bm25" },
                text,
                filtered,
            });
        }
    }
    let mut query_file =
        BufWriter::new(File::create(out.join(format!("queries-{n}.tsv"))).unwrap());
    writeln!(
        query_file,
        "category\tquery\ttype_filter\tdate_field\tstart\tend"
    )
    .unwrap();
    for q in &queries {
        writeln!(
            query_file,
            "{}\t{}\t{}",
            q.kind,
            q.text,
            if q.filtered {
                "bug\tcreated_at\t2026-05-01\t2026-09-01"
            } else {
                "\t\t\t"
            }
        )
        .unwrap();
    }
    query_file.flush().unwrap();
    drop(vocabulary);
    let baseline = memory();
    let indexer = RealIndexer::new();
    let before = Instant::now();
    rebuild_index(&vault, &indexer).unwrap();
    let rebuild_seconds = before.elapsed().as_secs_f64();
    let after = memory();
    eprintln!(
        "{n}: rebuild={rebuild_seconds:.3}s rss={} peak={}",
        after.0, after.1
    );
    if wait {
        eprintln!("{n}: rebuild complete; enter measured query limit (0 = all generated queries, authorized 100-query sample)");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        limit = line.trim().parse().expect("explicit query limit required");
    }
    let index = Index::new(
        &vault,
        &indexer,
        "bench",
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
    );
    let mut raw = BufWriter::new(File::create(out.join(format!("latencies-{n}.csv"))).unwrap());
    writeln!(raw, "query_index,category,milliseconds,result_count,candidate_generation_ms,authoritative_reads_ms,revalidation_ms,response_building_ms,candidates,reads,eligible").unwrap();
    let measured = if limit == 0 {
        queries.len()
    } else {
        limit.min(queries.len())
    };
    let mut durations = Vec::new();
    for (i, q) in queries.iter().take(measured).enumerate() {
        let filters = if q.filtered {
            SearchFilters {
                type_filter: Some(domain::NoteType::Bug),
                date_filter: Some((
                    TimeField::CreatedAt,
                    TimeExpression::Explicit {
                        start: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
                        end: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                    },
                )),
                ..Default::default()
            }
        } else {
            SearchFilters::default()
        };
        let start = Instant::now();
        let (response, profile) = index.search_profiled(&q.text, filters, 10).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let count = match response.outcome {
            SearchOutcome::NotPresent => 0,
            SearchOutcome::Matches { rest, .. } => 1 + rest.len(),
        };
        durations.push(ms);
        writeln!(
            raw,
            "{i},{},{ms:.6},{count},{:.6},{:.6},{:.6},{:.6},{},{},{}",
            q.kind,
            profile.candidate_generation.as_secs_f64() * 1000.0,
            profile.authoritative_reads.as_secs_f64() * 1000.0,
            profile.revalidation.as_secs_f64() * 1000.0,
            profile.response_building.as_secs_f64() * 1000.0,
            profile.candidates,
            profile.reads,
            profile.eligible
        )
        .unwrap();
        raw.flush().unwrap();
        if (i + 1) % 25 == 0 {
            eprintln!("{n}: queried {}/{measured}", i + 1);
        }
    }
    durations.sort_by(f64::total_cmp);
    let percentile =
        |p: f64| durations[((durations.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    let summary = format!("n,rebuild_seconds,rss_baseline,rss_populated,rss_delta,peak_baseline,peak_populated,original_protocol_queries,authorized_queries,measured_queries,p50_ms,p95_ms\n{n},{rebuild_seconds:.6},{},{},{},{},{},{},{},{},{:.6},{:.6}\n", baseline.0, after.0, after.0.saturating_sub(baseline.0), baseline.1, after.1, n/4, queries.len(), measured, percentile(0.50), percentile(0.95));
    fs::write(out.join(format!("summary-{n}.csv")), &summary).unwrap();
    println!("{summary}");
    eprintln!("synthetic vault retained at {}", root.display());
}
