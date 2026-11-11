# Grammar-conformance matrix

Quality & Semantic Reliability Hardening, Phase 1, deliverable 2.

## Purpose

`source -> AST shape -> extracted representation -> graph semantics`
must be verifiable, not assumed. This document is the human-readable
companion to `crates/codegraph-index/tests/grammar_conformance.rs`,
whose 34 tests are the actual, runnable, grammar-version-regression
gate — every cell below cites the exact test that confirms it. This
document is **maintained by hand**, not generated; keeping it in sync
with the test file as the test file grows is a residual limitation of
this Phase 1 pass (see `docs/research/findings/HARDENING-PHASE-1.md`
for the full list of residual risks).

Coverage is NOT exhaustive over each grammar's own productions — it is
prioritized to the shapes this project's own extraction/resolution
code actually depends on: symbol identity, `Calls`, `Contains`,
`Extends`, `Implements`, `Instantiates`, `TestsFor`, `Imports`. A cell
marked **N/A** means the category does not meaningfully apply to that
language in this pipeline's own scope (e.g. CSS/HTML have no call
graph).

## Matrix

| Category | Python | Rust | C# | C++ | JS | TS/TSX |
|---|---|---|---|---|---|---|
| **Class/type declaration** | `class_definition` | `struct_item`/`enum_item`/`trait_item` | `class_declaration`/`interface_declaration` (distinct kinds) | `class_specifier`/`struct_specifier` | `class_declaration` | `class_declaration`/`interface_declaration` |
| **Function declaration** | `function_definition` | `function_item` | `method_declaration` | `function_definition`, name inside `function_declarator` | `function_declaration`/`method_definition` | same as JS |
| **Method vs. free function** | SAME kind as function (`function_definition`) — grammar does not distinguish | SAME kind as free function (`function_item`) — grammar does not distinguish | SAME kind (`method_declaration`) whether in `class_declaration` or `interface_declaration` | **DIFFERENT name-child kind**: free function's name is `identifier`, a method's is `field_identifier`, inside the SAME `function_declarator` wrapper | N/A (methods are object/class properties, `property_identifier`) | same as JS |
| **Symbol name extraction** | direct `identifier` child, safe (only one such child exists on these node kinds) | direct `identifier` (functions) / `type_identifier` (types) child, safe | **UNSAFE positionally** — `method_declaration` has TWO direct `identifier` children (return type + name) when the return type is simple/non-generic; MUST use the named `name` field, never a positional scan (B2-ID-009) | name lives one level deeper inside `function_declarator`; an out-of-class qualified definition's name position is `qualified_identifier` (a DIFFERENT kind than `identifier`/`field_identifier`), so the current extractor correctly OMITS it rather than misnaming it | `property_identifier` for methods, `identifier` for functions/classes | same as JS |
| **Calls — plain/bare** | `call`, callee is direct `identifier` child | `call_expression`, callee is direct `identifier` child | `invocation_expression`, callee is direct `identifier` OR `generic_name` child | `call_expression`, callee is direct `identifier`, `template_function`, or `qualified_identifier` child | `call_expression`, callee is direct `identifier` child | same as JS |
| **Calls — receiver-qualified** | callee is `attribute` (`obj.foo`), resolved by its LAST `identifier` child | N/A (method calls go through `.`, same `call_expression` shape as a qualified path expression — not separately probed in this pass) | callee is `member_access_expression`, whose LAST name-kind child (`identifier` or `generic_name`) is the real method name | callee is `field_expression`, whose LAST name-kind child (`field_identifier` or `template_method`) is the real method name | callee is `member_expression`, whose LAST `property_identifier` child is the real method name | same as JS |
| **Calls — generic/template** | N/A (no generics at the call-site syntax level) | N/A (generic functions are called with the same `call_expression` shape; type arguments are not usually written at the call site) | bare: callee is a direct `generic_name` child of `invocation_expression`; qualified: callee is `member_access_expression` ending in a `generic_name` child. Confirmed exactly TWO direct children of the `generic_name`/`template_function` wrapper (a bare name, then the `<...>` clause) — no competing same-kind sibling, so a positional "first matching-kind child" search IS safe here (B2-ID-011, unlike B2-ID-009's own collision) | bare: `template_function` child of `call_expression`; qualified: `field_expression` ending in `template_method` (B2-ID-011) | N/A | N/A |
| **Calls — namespace/module-qualified** | module-qualified call (`module.func()`) parses as the SAME `attribute`-callee shape as a receiver-qualified method call — not separately distinguished by this grammar | path-qualified call (`module::func()`) is a distinct shape, not probed in this pass | N/A (C# uses `using`, not a call-site namespace qualifier syntax) | `ns::foo()` is a `qualified_identifier` callee — a DIFFERENT kind than any of `callee_name`'s own recognized match arms (`identifier`/`generic_name`/`template_function`/`attribute`/`field_expression`/`member_expression`/`member_access_expression`); **confirmed invisible to extraction, the new residual limitation B2-ID-011's own independent review found**, orthogonal to the generic/template fix | N/A | N/A |
| **Macro/attribute call opacity** | N/A | a macro invocation's own arguments are an opaque `token_tree` to tree-sitter — a call wrapped inside one (`assert_eq!(add(1,2), 3)`) is NEVER a real `call_expression` node, confirmed as the documented macro-call-opacity gap's exact grammar root cause | N/A | N/A | N/A | N/A |
| **Inheritance base-list syntax** | `argument_list` of `identifier`/`attribute` children on `class_definition` — syntactically can express multiple inheritance, always means `Extends` (Python has no separate interface concept) | N/A (trait implementation is `impl Trait for Type`, a SEPARATE non-symbol structural fact, `ImplFact`/B2-ID-008 — never a base-list) | `base_list`, a flat list of `identifier` children that does NOT syntactically distinguish a base class from an implemented interface — `resolve_extends_implements` falls back to the target's own resolved `SymbolKind` (`Interface` -> `Implements`, else `Extends`) | `base_class_clause` containing `type_identifier` children, likewise does NOT distinguish class from interface (C++ has no `interface` keyword) — same kind-based fallback as C# | `class_heritage` with ONLY an `extends` clause — plain JS has no `implements` keyword at all, so every heritage name is unconditionally `Extends` | `class_heritage` can carry BOTH `extends_clause` AND `implements_clause` as separate, syntactically unambiguous children — the clause tag is authoritative and must never be overridden by a target-kind lookup (a real historical defect this project already fixed) |
| **`partial`/fragmented declarations** | N/A | N/A | `partial` is a fixed lexical terminal token inside a `modifier` node — confirmed the combined `kind == "partial" && text == "partial"` check in this project's own code can never diverge, since no valid parse can produce that token with different text (B2-ID-010) | N/A | N/A | N/A |
| **Generic/type-parameter list** | N/A (runtime-only generics via `typing`) | `type_parameters` on the relevant item | `type_parameter_list`, its own distinct, unambiguous node kind with no competing sibling at the same position (safe for positional search, unlike `method_declaration`'s own name/return-type collision) | `template_parameter_list` (not separately probed in this pass) | N/A | `type_parameters` (not separately probed in this pass) |
| **Namespace declaration** | N/A (modules ARE the namespace, 1:1 with file path) | `mod` item (not separately probed in this pass) | THREE distinct forms, confirmed: `file_scoped_namespace_declaration` (`namespace X;`, applies to everything textually after it in the file), `namespace_declaration` (`namespace X { ... }`, block-scoped, can nest), and the global namespace (no declaration at all) — all three must canonicalize to one comparable representation (B2-ID-010's own `namespace_of`) | `namespace_definition` (not separately probed in this pass) | N/A | N/A |
| **Imports** | `import_statement` and `import_from_statement` are DISTINCT node kinds | `use_declaration` (not separately probed in this pass) | `using_directive` (not separately probed in this pass beyond B2-ID-010's own namespace work) | `#include` is a preprocessor directive, a fundamentally different mechanism than every other language's import syntax (not separately probed in this pass) | `import_statement` (ESM) (not separately probed in this pass) | same as JS, plus TS-specific `import type` (not separately probed in this pass) |
| **Field/instance-attribute assignment** | `self.x = ...` is a specific `assignment` shape whose LEFT child is an `attribute` node whose own first child is literally `self` — this exact shape, not a generic "any assignment," is what `Field` extraction recognizes | struct fields are declared, not assigned — a different mechanism entirely (not separately probed in this pass) | `field_declaration` (confirmed NOT currently walked by `extract/csharp.rs` at all — a known, documented extraction-coverage gap) | member variable declarations inside a `field_declaration_list` (not separately probed in this pass) | N/A (no static field declaration syntax at the class-body level in plain JS the same way) | TS class property declarations (not separately probed in this pass) |
| **Destructured/bound parameters** | N/A (Python has no destructuring-parameter syntax at this level) | pattern-matched function parameters exist but were not probed in this pass | N/A | N/A | a destructured parameter (`function Foo({ x })`) is a real, distinguishable `object_pattern` node inside `formal_parameters` — **confirmed the grammar does NOT hide this information; CRC-002's own false positive is a resolver gap (no local-binding/shadowing check), never a parser limitation** | same as JS |
| **JSX composition** | N/A | N/A | N/A | N/A | N/A (plain JS has no JSX) | `jsx_self_closing_element`/`jsx_element` are real, distinct node kinds the grammar fully supports — this pipeline does not currently model JSX composition as any typed edge at all (the documented JSX-composition gap is a MISSING CAPABILITY, not a parsing limitation) |

## Explicitly out of this pass's scope

- **CSS, HTML**: neither has a meaningful `Calls`/`Extends`/`Implements`/`Instantiates` concept in this pipeline's own domain model. HTML's `element_with_id` synthetic shape (used for some extraction) and CSS selector/rule shapes are not covered here — a future pass should add them only if a concrete finding motivates it, per this project's own evidence-before-claims discipline (no speculative grammar coverage).
- **Rust's `mod`/path-qualified calls, C++'s `template_parameter_list`/`namespace_definition`, TypeScript's `type_parameters`/`import type`, JS/TS ESM `import_statement`'s own internal shape**: named above as "not separately probed in this pass" — real gaps in this matrix's own completeness, not claims that these shapes are safe. Flagged explicitly as residual scope for a future hardening pass rather than silently omitted.
- **Constructors**: C#'s `constructor_declaration` is confirmed (via `csharp_declarations.rs`'s own existing code, not re-probed here) to correctly expose a named `name` field matching the class name — not independently re-verified by a NEW test in this pass, since no defect has ever been found there. Python's `__init__`/Rust's associated functions are ordinary `function_definition`/`function_item` nodes, already covered above.

## How to use this matrix

- Before trusting a NEW extraction/resolution shape for any of the
  languages above, probe it directly against the real grammar first
  (the pattern every cell here and every corresponding test in
  `grammar_conformance.rs` already uses) — never assume a shape from
  memory, prior language experience, or official grammar
  documentation (which this project's own history has found to not
  always state the exact shape precisely enough, e.g. the half-open
  byte-span convention).
- If `grammar_conformance.rs`'s own tests start failing after a
  `tree-sitter-*` grammar crate version bump, treat that as a release
  blocker, not a flaky test — it means a node kind this project's
  production code depends on has changed shape upstream.
