//! B2 Task 4: C# symbol extraction, node kinds confirmed against a real
//! parse. `interface_declaration`/`class_declaration` name via a direct
//! `identifier` child — each has exactly one such child (the type's own
//! name; a base/heritage list's names live inside a separate `base_list`
//! child, never as a bare sibling), so a positional "first `identifier`
//! child" search is safe for these two. A method is `method_declaration`
//! whether found inside a `class_declaration`'s or an
//! `interface_declaration`'s `declaration_list` — same node kind either
//! way, so `kind_hint` does not itself distinguish "interface method
//! signature" from "class method with a body."
//!
//! # B2-ID-009: `method_declaration` must use the grammar's own `name`
//! field, never a positional child search
//!
//! Unlike `class_declaration`/`interface_declaration`, a
//! `method_declaration` node can have TWO direct children of kind
//! `identifier`: the return type (when it is a simple, non-generic,
//! non-`predefined_type` named type, e.g. `PolicyBuilder Handle()`) and
//! the method's own name, in that order. A positional "first identifier
//! child" search — this module's own approach before this fix — finds
//! the return type instead of the name whenever that shape occurs,
//! silently misnaming the candidate as its own return type. Confirmed
//! via a real-repository reproduction
//! ([B2-ID-009](../../../../docs/research/findings/B2-ID-009.md),
//! `App-vNext/Polly`'s `Policy.HandleSyntax.cs`:
//! `public static PolicyBuilder Handle<TException>()` extracted with
//! name `"PolicyBuilder"`, not `"Handle"`).
//!
//! The grammar itself already disambiguates this unambiguously: a real
//! `method_declaration` node exposes a named `name` field (and a
//! separate named `returns` field), confirmed via direct
//! `child_by_field_name` probes against this workspace's real compiled
//! `tree-sitter-c-sharp` grammar across every return-type shape this
//! project's own fix-authorization explicitly required coverage for —
//! `void`, a primitive (`predefined_type`), a simple named type
//! (`identifier`, the exact collision shape), a qualified name
//! (`qualified_name`, e.g. `System.String`), a generic name
//! (`generic_name`, e.g. `PolicyBuilder<T>`, `Task<MyType>`), a
//! nullable type (`nullable_type`), an array type (`array_type`), a
//! tuple type (`tuple_type`), inside both a `class_declaration` and an
//! `interface_declaration` body, and for an explicit interface
//! implementation (`MyType IFoo.Foo()`, name field correctly yields
//! `"Foo"`, not `"IFoo.Foo"` — a separate, pre-existing, NOT-in-scope
//! naming-granularity property of this extractor, unrelated to this
//! fix). `method_declaration` is therefore read via
//! `named_by_name_field` (field-based), never via the positional
//! `named_by_child_kind` this module still uses for
//! `class_declaration`/`interface_declaration`, where it remains safe.

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    match node.kind() {
        "interface_declaration" | "class_declaration" => {
            if let Some(candidate) = named_by_child_kind(node, source, "identifier") {
                out.push(candidate);
            }
        }
        "method_declaration" => {
            if let Some(candidate) = named_by_name_field(node, source) {
                out.push(candidate);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

fn named_by_child_kind(node: tree_sitter::Node, source: &[u8], name_kind: &str) -> Option<Candidate> {
    let mut cursor = node.walk();
    let name_node = node.children(&mut cursor).find(|c| c.kind() == name_kind && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    build_candidate(node, source, name)
}

/// Reads a `method_declaration`'s name via the grammar's own named
/// `name` field — never a positional/kind-based child search — per
/// B2-ID-009. Returns `None` (omits the candidate) if the field is
/// absent or `MISSING`, the same "prefer omit over false certainty"
/// policy every other extractor in this project already follows.
fn named_by_name_field(node: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let name_node = node.child_by_field_name("name").filter(|n| !n.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    build_candidate(node, source, name)
}

fn build_candidate(node: tree_sitter::Node, source: &[u8], name: String) -> Option<Candidate> {
    let kind_hint = node.kind().to_string();
    let span = ByteSpan::from_node(&node);
    let signature_text = node_text(&node, source).to_string();
    Some((name, kind_hint, span, signature_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{CSharpParser, LanguageParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = CSharpParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn a_single_class_is_extracted() {
        let candidates = extract_from(b"class C {}\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "C");
        assert_eq!(candidates[0].1, "class_declaration");
    }

    #[test]
    fn multiple_top_level_symbols_are_all_extracted() {
        let source = b"interface I {}\nclass C {}\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["I", "C"]);
    }

    #[test]
    fn a_class_with_a_nested_method_extracts_both() {
        let source = b"class C { public void M() {} }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["C", "M"]);
    }

    #[test]
    fn an_interface_with_a_method_signature_extracts_both() {
        let source = b"interface I { void M(); }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["I", "M"]);
    }

    #[test]
    fn a_file_with_no_symbols_yields_nothing() {
        let candidates = extract_from(b"// just a comment\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_the_well_formed_symbols() {
        // The malformed expression `int x = (;` inside M's body is
        // locally contained by tree-sitter's error recovery (confirmed
        // by inspecting the actual parse tree) — it does not swallow the
        // enclosing class or the next, independent top-level class.
        let source = b"class Broken {\n    void M() { int x = (; }\n}\nclass WellFormed {}\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn multiline_signature_is_captured_verbatim() {
        let source = b"class C {\n    public void M(\n        int a,\n        int b\n    ) {}\n}\n";
        let candidates = extract_from(source);
        let method = candidates.iter().find(|c| c.0 == "M").unwrap();
        assert!(method.3.contains('\n'));
    }

    #[test]
    fn span_matches_the_exact_source_bytes() {
        let source = b"class Exact {}\n";
        let candidates = extract_from(source);
        let (_, _, span, signature_text) = &candidates[0];
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
    }

    /// B2-ID-009 regression coverage: a `method_declaration` must
    /// always extract its own NAME, never its return type, across
    /// every return-type shape this project's own fix-authorization
    /// required — confirmed via direct grammar probes before this fix
    /// was written, not assumed. Each case also asserts the absence of
    /// a bogus candidate named after the return type, since the
    /// original bug produced exactly that extra wrong name rather than
    /// merely omitting the real one.
    mod b2_id_009_method_name_vs_return_type {
        use super::*;

        fn method_names(source: &[u8]) -> Vec<String> {
            extract_from(source).into_iter().filter(|c| c.1 == "method_declaration").map(|c| c.0).collect()
        }

        #[test]
        fn case_1_void_return() {
            assert_eq!(method_names(b"class C { void Foo() {} }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_2_primitive_return() {
            assert_eq!(method_names(b"class C { int Foo() => 1; }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_3_simple_named_return_type_is_the_original_bug_shape() {
            // The exact real-repository shape from B2-ID-009
            // (App-vNext/Polly's Policy.HandleSyntax.cs): a simple,
            // non-generic, non-predefined named return type sharing
            // the `identifier` node kind with the method's own name.
            let names = method_names(b"class C { PolicyBuilder Handle() => null; }\n");
            assert_eq!(names, vec!["Handle"], "must extract the method name, not its return type \"PolicyBuilder\"");
            assert!(!names.contains(&"PolicyBuilder".to_string()), "must not produce a bogus candidate named after the return type");
        }

        #[test]
        fn case_4_qualified_return_type() {
            assert_eq!(method_names(b"class C { System.String Foo() => null; }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_5_generic_return_type() {
            let names = method_names(b"class C { PolicyBuilder<T> Handle<T>() => null; }\n");
            assert_eq!(names, vec!["Handle"]);
        }

        #[test]
        fn case_6_nullable_return_type() {
            assert_eq!(method_names(b"class C { MyType? Foo() => null; }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_7_array_return_type() {
            assert_eq!(method_names(b"class C { MyType[] Foo() => null; }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_8_tuple_return_type() {
            assert_eq!(method_names(b"class C { (int, int) Foo() => (1, 2); }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_9_task_generic_async_return_type() {
            assert_eq!(method_names(b"class C { async Task<MyType> Foo() => null; }\n"), vec!["Foo"]);
        }

        #[test]
        fn case_10_generic_method_with_simple_named_return_type() {
            // The real Polly shape again, explicit about the method
            // itself (not just its return type) being generic.
            let names = method_names(b"class C { PolicyBuilder Handle<TException>() => null; }\n");
            assert_eq!(names, vec!["Handle"]);
        }

        #[test]
        fn case_11_interface_method_with_simple_named_return_type() {
            let names = method_names(b"interface I { PolicyBuilder Handle(); }\n");
            assert_eq!(names, vec!["Handle"]);
        }

        #[test]
        fn case_12_overloads_with_the_same_name_both_extract_correctly() {
            // The real Polly shape: two overloads sharing one name,
            // both with the same simple-named-return-type collision
            // risk, must BOTH extract as "Handle", never collapse to
            // one or misname either.
            let source = b"class C {\n    public static PolicyBuilder Handle() => null;\n    public static PolicyBuilder Handle(int x) => null;\n}\n";
            let names = method_names(source);
            assert_eq!(names, vec!["Handle", "Handle"]);
        }

        #[test]
        fn case_13_return_type_identifier_equal_to_a_real_symbol_name() {
            // The return type's own name ("Base") happens to collide
            // with a REAL class also present in the corpus. Before the
            // fix, this would produce a spurious THIRD candidate named
            // "Base" (the misread return type) in addition to the real
            // "Base" class and the real "Make" method.
            let source = b"class Base {}\nclass C { Base Make() => null; }\n";
            let candidates = extract_from(source);
            let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(names, vec!["Base", "C", "Make"], "must not produce an extra spurious \"Base\" candidate from the method's return type");
        }

        #[test]
        fn case_14_multiple_modifiers_before_a_simple_named_return_type() {
            let names = method_names(b"class C { public static virtual PolicyBuilder Handle() => null; }\n");
            assert_eq!(names, vec!["Handle"]);
        }

        #[test]
        fn class_and_interface_declaration_names_are_unaffected_by_this_fix() {
            // Scope-discipline check: class_declaration/interface_declaration
            // still use the positional named_by_child_kind path, untouched
            // by this fix, and must still extract correctly.
            let source = b"interface IFoo {}\nclass Derived : IFoo {}\n";
            let candidates = extract_from(source);
            let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
            assert_eq!(names, vec!["IFoo", "Derived"]);
        }
    }
}
