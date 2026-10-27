// `git2` (via `libgit2-sys`) needs several Windows system libraries
// (advapi32: registry + crypto + SID APIs used by libgit2's fs_path, sysdir
// and rand modules) that are not always pulled in automatically by the
// MSVC linker invocation `cargo`/`rustc`/`rustdoc` build. Declaring it here,
// rather than only via `.cargo/config.toml` rustflags, makes the link
// succeed for every build mode that compiles this crate — including
// `cargo test --doc`, which does not apply `.cargo/config.toml`'s
// `rustflags` the same way an ordinary test binary does.
fn main() {
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        println!("cargo:rustc-link-lib=dylib=advapi32");
    }
}
