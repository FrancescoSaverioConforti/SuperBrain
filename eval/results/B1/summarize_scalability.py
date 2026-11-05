"""Validate measured benchmark artifacts; never extrapolate missing scales."""
import argparse
import csv
import hashlib
import json
import math
from collections import Counter
from pathlib import Path
import shutil
import sys

if sys.flags.optimize:
    raise RuntimeError('Benchmark validation requires Python assertions enabled')

SCALES = (100, 1000, 10000, 100000)
PHASES = ("candidate_generation_ms", "authoritative_reads_ms", "revalidation_ms", "response_building_ms")


def rows(path, delimiter=","):
    with path.open(encoding="utf-8", newline="") as stream:
        return list(csv.DictReader(stream, delimiter=delimiter))


def quantile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_csv(path, records):
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(records[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(records)


def summarize(source, destination):
    summaries, phases, artifacts = [], [], []
    # Validate ALL scales before publishing the aggregate. Missing files are
    # errors; there is no imputation, estimated percentile or skipped scale.
    for n in SCALES:
        summary_path = source / f"summary-{n}.csv"
        summary = rows(summary_path)
        assert len(summary) == 1, (n, "one summary row required")
        summary = summary[0]
        queries_path, latency_path = source / f"queries-{n}.tsv", source / f"latencies-{n}.csv"
        queries, latencies = rows(queries_path, "\t"), rows(latency_path)
        assert len(queries) == len(latencies) == 100, (n, "100 actual measurements required")
        expected = Counter(exact=40, bm25=40, filtered=20)
        assert Counter(q["category"] for q in queries) == expected, (n, "query mix")
        assert Counter(q["category"] for q in latencies) == expected, (n, "measured mix")
        assert int(summary["n"]) == n
        assert int(summary["original_protocol_queries"]) == n // 4
        assert int(summary["authorized_queries"]) == int(summary["measured_queries"]) == 100
        for i, (query, latency) in enumerate(zip(queries, latencies)):
            assert int(latency["query_index"]) == i and query["category"] == latency["category"]
            total = float(latency["milliseconds"])
            measured_phases = [float(latency[key]) for key in PHASES]
            assert math.isfinite(total) and total >= 0
            assert all(math.isfinite(value) and value >= 0 for value in measured_phases)
            # CSV values are rounded to 6 decimals; 0.001ms permits only
            # serialization rounding, not overlapping phase measurements.
            assert sum(measured_phases) <= total + 0.001, (n, i, "overlapping phases")
            candidates, reads, eligible = (int(latency[key]) for key in ("candidates", "reads", "eligible"))
            assert 0 <= int(latency["result_count"]) <= min(eligible, 10)
            assert 0 <= eligible <= reads == candidates
        totals = [float(row["milliseconds"]) for row in latencies]
        for field, fraction in (("p50_ms", 0.5), ("p95_ms", 0.95)):
            assert abs(float(summary[field]) - quantile(totals, fraction)) < 0.0001, (n, field)
        baseline, populated, delta, peak_before, peak_after = (
            int(summary[key]) for key in ("rss_baseline", "rss_populated", "rss_delta", "peak_baseline", "peak_populated")
        )
        assert min(baseline, populated, delta, peak_before, peak_after) >= 0
        assert delta == max(0, populated - baseline)
        assert peak_after >= max(populated, peak_before)
        assert peak_before >= baseline
        assert math.isfinite(float(summary["rebuild_seconds"])) and float(summary["rebuild_seconds"]) > 0
        summaries.append(summary)
        for category in ("all", "exact", "bm25", "filtered"):
            selected = [row for row in latencies if category == "all" or row["category"] == category]
            wall = sum(float(row["milliseconds"]) for row in selected)
            phase = {key: sum(float(row[key]) for row in selected) for key in PHASES}
            phases.append({
                "n": n, "category": category, "measured_queries": len(selected),
                "wall_ms": wall, **phase, "residual_ms": max(0, wall - sum(phase.values())),
                "p50_ms": quantile([float(row["milliseconds"]) for row in selected], 0.5),
                "p95_ms": quantile([float(row["milliseconds"]) for row in selected], 0.95),
            })
        corpus = source / f"corpus-{n}.mdstream"
        artifacts.append({
            "n": n, "corpus_sha256": sha256(corpus), "corpus_bytes": corpus.stat().st_size,
            "queries_sha256": sha256(queries_path), "latencies_sha256": sha256(latency_path),
            "summary_sha256": sha256(summary_path),
        })
    destination.mkdir(parents=True, exist_ok=True)
    for n in SCALES:
        for stem, extension in (("summary", "csv"), ("latencies", "csv"), ("queries", "tsv")):
            filename = f"{stem}-{n}.{extension}"
            shutil.copyfile(source / filename, destination / filename)
    write_csv(destination / "scalability.csv", summaries)
    write_csv(destination / "phase-totals.csv", phases)
    manifest = {
        "status": "authorized_sample_measured_and_validated",
        "original_full_protocol_executed": False,
        "queries_per_scale": 100, "composition": dict(exact=40, bm25=40, filtered=20),
        "percentile_method": "nearest rank, one measured run per scale, no confidence interval",
        "memory_scope": "process working set and lifetime peak; RSS delta is approximate attribution",
        "artifacts": artifacts,
    }
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    summarize(args.source, args.destination)
