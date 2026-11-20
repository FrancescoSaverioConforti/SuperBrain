//! M6.3 Phase 3's own explicit prohibition: "Do not use nested
//! `Handle::block_on`." Checked structurally (never relying on "it
//! compiled and passed" alone, since a nested `block_on` can deadlock
//! only under specific runtime-flavor/thread conditions that may not be
//! exercised by every test run) by scanning this crate's own real source
//! text for the literal pattern -- the same "parse the real file as
//! text" discipline `tests/dependency_boundary.rs` and
//! `mcp-client/tests/separation.rs` already use for their own structural
//! guarantees.

use std::path::PathBuf;

#[test]
fn no_source_file_in_this_crate_spells_block_on() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offending = Vec::new();
    for entry in std::fs::read_dir(&src_dir).expect("src dir must exist") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // Checks for actual CODE usage (a method call), never a bare
        // mention of the word -- this crate's own doc comments
        // legitimately document the prohibition by name (e.g. this
        // module's own top-level doc comment, and worker.rs's own
        // `_NO_NESTED_BLOCK_ON` regression anchor), which must not
        // false-positive against this structural guard.
        let has_block_on_call = text.lines().any(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                return false;
            }
            line.contains(".block_on(")
        });
        if has_block_on_call {
            offending.push(path);
        }
    }
    assert!(offending.is_empty(), "this crate's own worker must never call Handle::block_on (nested or otherwise) -- found the literal in: {offending:?}");
}
