# CommonJS Extraction Checkpoint — require()/exports.X/module.exports.X/Foo.prototype.X

**Status:** COMPLETE. All three authorized findings (B2-ID-013,
B2-ID-014, B2-ID-015) are FIXED, tested end-to-end through the real
`CodeGraphIndexer`, mutation-closed (0 genuine missed), confirmed
full/incremental-parity-safe, and remeasured on the real
`request/request` corpus against a genuinely clean (pre-fix,
separate-worktree-built) before baseline. One new Moderate finding
(B2-ID-016) opened during remeasurement — a real, distinct CommonJS
idiom outside this checkpoint's own authorized 4-point scope, not a
defect in this checkpoint's own fixes. No Critical/Important finding
at any point. A5/B2-EVAL-011 both remain unauthorized and untouched.

**Authorized by:** the user's own explicit directive following
B2-EVAL-010's approval, naming exactly four extraction shapes
(`require(...)`, `exports.X = function(){}`, `module.exports.X =
function(){}`, `Foo.prototype.X = function(){}`) and explicitly
forbidding A5/type-inference/points-to/data-flow/callback-graph/any
unrelated new JS feature.

**Branch/worktree:** `b2-code-graph`,
`C:\Users\fsave\Desktop\Python_code\SuperBrain\.worktrees\b2-code-graph`.
No merge, tag, or push. B2-EVAL-011 not started. A5 not started.

## Process note

A first fork dispatch for this checkpoint went off-scope mid-task,
returning an unrelated untracked-file-cleanup report instead of
executing the assigned work. That report was discarded entirely per
the user's own explicit instruction (recorded as Continuity rejection
`[x2gi]`) and is not referenced further. This report reflects a
second, independent execution that completed the assigned work
directly (not delegated to a further subagent).

## What was implemented

### 1. `require(...)` → `ImportFact` (B2-ID-013)

`imports.rs` gained `collect_js_require`, wired into `walk_for_imports`
alongside the existing ESM-only `collect_js_import`. Recognizes
`require(<string>)` (callee must be the bare `identifier` "require" —
`require_call_module_path` explicitly rejects a member-expression
callee like `foo.require(...)`, confirmed by a dedicated test) as the
direct RHS of a `variable_declarator`:
- Simple binding (`const x = require('./x')`) → one fact, bound name
  as `alias`.
- Aliased binding (`var y = require('./mod')`) → same shape.
- Destructured binding (`const { a, b } = require('./x')`) → one fact
  per destructured name, mirroring Python's own `from X import a, b`
  convention (never one fact for the whole pattern).
- Deliberately NOT recognized, per this checkpoint's own narrow scope:
  a bare `require('./x');` with no binding, and `require(...)` nested
  inside another call (`wrap(require('./x'))`) — both confirmed by
  dedicated negative tests.

### 2/3. `exports.X`/`module.exports.X` = function (B2-ID-014)

`extract/javascript.rs` gained `commonjs_property_function_assignment`,
recognizing an `assignment_expression` whose LHS is a
`member_expression` ending in a `property_identifier` and whose RHS is
a `function_expression`/`arrow_function`. The receiver must be
literally `exports` or `module.exports` (`is_exports_receiver`) — both
forms produce the identical new synthetic `kind_hint`
(`js_exports_property_function` → `SymbolKind::Function`, added to
`kind_mapping.rs`'s exhaustive table). A non-function RHS
(`exports.VERSION = "1.0.0"`) is correctly never a candidate — this
checkpoint's own scope is callable candidates, not general
property-export tracking.

### 4. `Foo.prototype.X` = function, with fail-closed ownership (B2-ID-015)

Same extraction function additionally recognizes `Foo.prototype.X =
function(){...}` via `is_prototype_receiver` (the receiver must be a
`member_expression` of the exact shape `Identifier.prototype`), a new
synthetic `kind_hint` (`js_prototype_method_assignment` →
`SymbolKind::Method`). The candidate's own `signature_text` is
deliberately the WHOLE assignment expression (not just the RHS
function), so a later resolution step can recover the owner name from
it — mirroring `resolve_instantiates`'s own "re-parse the candidate's
own signature_text at resolution time" pattern.

**Ownership** is a genuinely new resolver,
`edges::js_prototype_ownership::resolve_js_prototype_ownership`, not a
`resolve_contains` extension — `resolve_contains`'s own AST-span-
nesting mechanism is structurally inapplicable here (the method body
is a SIBLING top-level statement, never nested inside the constructor
function's own span). The new resolver re-parses each `Method`
candidate's own `signature_text`, recovers the receiver identifier
(`Foo`), and looks it up among same-file `Function | Class` candidates
via the existing `TargetLookup::find_by_name` + "exactly one candidate"
discipline already used throughout `edges/`. **Fail-closed, explicitly
confirmed on the real ambiguous-owner shape**: `var Foo =
require('./other'); Foo.prototype.bar = function(){}` — `Foo` here is
an imported binding, never a real `Function`/`Class` snapshot, so zero
candidates survive the filter and no edge is produced (not merely
asserted — a dedicated test drives this exact shape through the real
extractor). An ambiguous same-file name collision, and a same-named
Function in a DIFFERENT file, are both confirmed to fail closed too.
Reuses `EdgeKind::Contains` (no new edge kind) — the semantic claim is
identical to Class-contains-Method, resolved by name instead of span.

Wired into `resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`
— the real production entry point both the full and incremental
indexing paths call — so this is live everywhere, not a separate
code path.

## Golden corpus

Nine new fixtures under `tests/corpus/javascript/`: `commonjs_require_shapes.js`
+ `helper.js` (simple/aliased/destructured/package require), `commonjs_exports_property_function.js`
+ `commonjs_exports_property_function_caller.js` (exports.X/module.exports.X with a real
receiver-qualified cross-file call site), `commonjs_prototype_method.js`
(a real two-method Request-shaped constructor with an internal
`this.init(...)`-style call), `commonjs_prototype_ownership_ambiguous.js`
(the require()-aliased ambiguous-owner fail-closed case),
`commonjs_prototype_method_reassignment.js` (the re-assignment edge
case). Seven new end-to-end tests in `golden_corpus.rs`, all driven
through the real `CodeGraphIndexer::observe_commit`, not a
shortcut calling one stage in isolation.

## Validation

- **End-to-end:** all 7 new golden-corpus tests pass through the real
  indexer.
- **Full/incremental parity:** a new test,
  `full_equals_incremental_for_commonjs_prototype_method_ownership`,
  drives a real incremental edit (adding a second prototype method to
  an untouched constructor file) through `compare_full_incremental_state`
  — the SAME exact-multiset equivalence gate every other parity test
  in this project uses. Zero divergences. Also confirmed on the real
  `request/request` corpus directly (see Performance/Parity below).
- **Mutation testing:** 30 mutants across the diff of modified files
  (`imports.rs`, `extract/javascript.rs`, `kind_mapping.rs`,
  `edges/mod.rs`) — 26 caught, 4 unviable (`Default::default()`-style,
  not real gaps), 0 missed. 24 mutants on the new
  `js_prototype_ownership.rs` file — 1 genuine missed mutant found
  (an `&&`→`||` on the literal-"prototype"-text check, which would
  have treated ANY `Identifier.<anything>.X = function(){}` as a
  prototype assignment regardless of the middle property's actual
  text), fixed with a dedicated regression test
  (`a_member_expression_receiver_whose_property_is_not_literally_prototype_is_not_recognized`),
  re-verified 10/10 caught, 0 missed on re-run. **Combined final: 0
  genuine missed mutants across all new/changed code.**
- **Full test suite:** 635 lib tests (up from the pre-checkpoint 607 +
  this round's ~27 new unit tests) + 5 full/incremental equivalence +
  24 golden corpus + 34 grammar conformance + 12 ground-truth schema +
  10 reliability gates = 720 passing, 0 failures, 0 regressions.
- **Clippy:** zero warnings in any file this checkpoint touched. One
  PRE-EXISTING, unrelated clippy error pair in `agent_policy.rs`
  (`manual_contains` lint, lines 132/135) blocks a clean
  `cargo clippy --workspace -- -D warnings` run — confirmed via `git
  diff --stat` that this file has zero changes from this checkpoint's
  own work; not fixed here, out of scope, flagged for a future pass.

## Real-corpus remeasurement — `request/request`@`3c0cddc7c8eb60b470e9519da85896ed7ee0081e`

**Methodology note:** the first remeasurement attempt was discovered
to be invalid — the project's own "BEFORE" harness
(`b2_eval_010_js_commonjs.rs`) is an untracked file compiled against
whatever source is currently checked out, and this checkpoint's own
fixes were already live in the working tree, silently contaminating
the "before" run with the "after" code. Caught and corrected: a
genuinely clean before-baseline was built in a throwaway, detached
`git worktree` pinned at the pre-checkpoint commit (`f50c634`),
confirmed to reproduce B2-EVAL-010's own original numbers exactly (40
snapshots, 0 ImportFacts, 12 Calls edges, 0/27 prototype methods
visible), then deleted.

| Metric | BEFORE (genuine) | AFTER |
|---|---|---|
| Snapshots | 40 (all Function) | 96 (44 Function, 52 Method) |
| ImportFacts | 0 | 48 (all External) |
| Calls edges | 12 | 88 |
| CallAttempts (negative knowledge) | 68 | 215 |
| Contains edges | 0 | 52 |
| `Request.prototype.X` visible | 0/27 | 24/27 |
| `Request.prototype.X` correctly owned (of visible) | — | 24/24 (100%) |
| False edges (any case checked) | 0 | 0 |

The 3 invisible prototype methods (`init`, `start`,
`onRequestResponse`) are rejected by this pipeline's own pre-existing
`MAX_SIGNATURE_BYTES = 4096` bound — confirmed real (e.g. `init` spans
~443 lines) and unrelated to this checkpoint's own fixes, not a new
limitation.

**Ground-truth cross-file call sites** (the 4 originally cited in
B2-EVAL-010): `hawk.header(...)` at `request.js:1422` now resolves
end-to-end (`resolved_edges=1`) — the one case entirely inside this
checkpoint's own scope, confirming the fix works fully on real data.
The `jar`/`parse`-targeting call sites surfaced two further honest
findings, not defects: (1) `jar` now has a genuine real name
collision — `lib/cookies.js`'s `exports.jar` AND `request.js`'s own
`Request.prototype.jar` are both real, distinct, now-visible
declarations sharing one bare name, which this checkpoint's own fixes
correctly expose rather than create; (2) `index.js`'s own `request.jar
= function(){...}`/`request.cookie = ...` (property assignment onto a
plain local identifier, not `exports`/`module.exports`/`.prototype`)
is a genuinely different, fifth CommonJS idiom outside this
checkpoint's authorized scope — newly documented as **B2-ID-016**
(Moderate, not fixed, explicitly out of scope).

## Parity and performance (real corpus, fixes live)

Same commit pair as B2-EVAL-010's own prior measurement
(`be7882b0...`→`df346d8531...`, 3 real changed files incl.
`request.js`), via the project's own existing, reused
`b2_eval_009_cpp_parity`/`call_res_a4_perf_measurement` harnesses
(whole-repo scope, not the library-only scope used for the corpus
metrics table above):

- **Parity:** `compare_full_incremental_state` — zero divergences
  (full: 158 snapshots/197 edges/292 import_facts/498 call_attempts;
  incremental: 191 snapshots [benign lineage retention]/197 edges/292
  import_facts/498 call_attempts, identical on every multiset that
  matters).
- **Performance:** full-index median 205.26ms (min 198.07, max
  213.52ms, 10 runs); incremental median 61.14ms (min 59.65, max
  67.56ms). Edge-count stability 10/10 identical on both sides.
  Compared to B2-EVAL-010's own pre-checkpoint baseline on this same
  corpus (full ~211ms, incremental ~35ms): full-index time is
  essentially unchanged; incremental time increased by roughly
  26ms/+74% — attributable to the new `resolve_js_prototype_ownership`
  resolver's own re-parse-per-Method-candidate cost plus the new
  `require()` extraction walk. Not measured as a dedicated
  before/after isolation (this checkpoint's own scope did not require
  it), reported as the honest overall delta.

## Severity discipline

No Critical or Important finding was encountered at any point. The
one new finding (B2-ID-016) is Moderate — a real, distinct CommonJS
idiom outside the checkpoint's own authorized scope, not a defect in
the delivered fixes (every check confirms zero false edges throughout).

## Independent review

Performed sequentially (not wide parallel fan-out), covering: (1) the
extraction logic's exact AST-shape claims, re-confirmed against a
direct tree-sitter-javascript probe for all 4 real shapes plus the
ambiguous-owner and non-prototype-receiver edge cases; (2) the
ownership resolver's fail-closed discipline, re-confirmed by tracing
`prototype_owner_name`'s own logic against the one real missed-mutant
case and its fix; (3) whether the real-corpus remeasurement's own
BEFORE baseline was genuinely uncontaminated (the self-caught
contamination issue above, independently re-verified by reproducing
B2-EVAL-010's exact original numbers from the clean worktree build);
(4) whether B2-ID-016 was correctly scoped as a new, out-of-scope
finding rather than silently implemented or silently ignored; (5)
mutation/test/clippy claims, spot-checked against the actual
`mutants.out/caught.txt`/`missed.txt` files and `cargo test` output
rather than only the implementer's own summary.

**Verdict: APPROVE.** No Critical/Important issue found. No
corrections required to this report's substance — the one process
error (the contaminated first before-baseline attempt) was caught and
fixed by the implementer's own verification discipline before being
reported, not left in the final numbers.

## GO/NO-GO for this checkpoint

**GO — checkpoint CLOSED.** All three authorized findings fixed,
tested, mutation-closed, parity-confirmed, and remeasured on real
data. No Critical/Important defect. One new Moderate finding
documented (B2-ID-016), correctly left unfixed per scope discipline.

## GO/NO-GO for B2-EVAL-011

**GO, pending the user's own explicit authorization — not
auto-started per this checkpoint's own directive.** The CommonJS
domain's own extraction-layer gaps that B2-EVAL-010 found are now
substantially closed (89% of real prototype methods, both export
idioms, require() imports); remaining real gaps are either pre-existing
and unrelated (`SignatureTooLong`) or newly scoped out (B2-ID-016) —
none block moving to a new domain.

## Recommendation on CALL-RES-A5

**Still not justified — same conclusion as every prior checkpoint in
this series, now reinforced by this round's own evidence.** Every
real residual gap this checkpoint's own remeasurement surfaced
(`SignatureTooLong` rejections, B2-ID-016's own out-of-scope idiom,
the real `jar`/`Request.prototype.jar` name collision) is an
EXTRACTION-layer or SCOPE question, not a call-resolution-quality one
A5 (type/points-to-aware resolution) would address. The real `jar`
name collision is the closest candidate for an A5-shaped argument —
but it is a genuine, honest ambiguity between two real, independently-
declared functions sharing one bare name across files with no
namespace/module-scoping information this tool has access to; A5
would need real cross-module binding resolution (knowing that
`cookies.jar` in one file and the receiver-qualified call site's own
`cookies` variable are the SAME `require()`-bound reference) to
disambiguate it correctly — which is closer to CALL-RES-A4's own
import-alias mechanism's natural next extension (teaching it to
narrow Ambiguous candidates via a resolved import alias, not merely
zero-candidate cases) than to A5's type-inference scope. **Conclusion:
A5 remains unauthorized and unjustified by this round's evidence.**
