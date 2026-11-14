"""Task 3 Step 4: assign dev/calibration/test split (~50/25/25),
stratified across (category, difficulty) cells, deterministically."""
import json
from collections import defaultdict
from pathlib import Path

DRAFT = Path(__file__).resolve().parent / "queries-draft.jsonl"

rows = [json.loads(l) for l in open(DRAFT, encoding="utf-8") if l.strip()]

cells = defaultdict(list)
for r in rows:
    cells[(r["category"], r["difficulty"])].append(r)

# Deterministic largest-remainder allocation per cell, targeting 50/25/25,
# so small cells don't all collapse into one split by pattern-phase luck.
SPLIT_TARGETS = {"dev": 0.5, "calibration": 0.25, "test": 0.25}
# Running totals, adjusted greedily cell-by-cell so the overall split
# stays close to target even though each cell is tiny.
running = {"dev": 0, "calibration": 0, "test": 0}

for key in sorted(cells):
    items = cells[key]
    items.sort(key=lambda r: r["id"])  # stable, reproducible order
    n = len(items)
    total_so_far = sum(running.values())
    for i, r in enumerate(items):
        total_assigned = sum(running.values())
        # pick the split currently furthest BELOW its target share
        deficits = {
            s: SPLIT_TARGETS[s] * (total_assigned + 1) - running[s]
            for s in SPLIT_TARGETS
        }
        chosen = max(deficits, key=lambda s: deficits[s])
        r["split"] = chosen
        running[chosen] += 1

counts = defaultdict(int)
for r in rows:
    counts[r["split"]] += 1
print("split counts:", dict(counts))

with open(DRAFT, "w", encoding="utf-8", newline="\n") as f:
    for r in rows:
        f.write(json.dumps(r) + "\n")

# per-split category/difficulty coverage check
from collections import Counter
for split in ["dev", "calibration", "test"]:
    split_rows = [r for r in rows if r["split"] == split]
    cats = set(r["category"] for r in split_rows)
    diffs = set(r["difficulty"] for r in split_rows)
    print(f"{split}: {len(split_rows)} queries, categories={sorted(cats)}, difficulties={sorted(diffs)}")
