//! Minimal CLI approval surface (B5.1 Task 15): `cargo run --example
//! approve_cli -- <vault-root> <journal-path> list|approve <id>|reject <id> <reason>`.
//!
//! A thin wrapper over `brain_api::run_cli` — all the real logic (command
//! parsing, trusted-context construction, dispatch to `approve_proposal`/
//! `reject_proposal`/`list_proposals`) lives there, tested in
//! `tests/cli_approval_e2e.rs` without spawning this binary. This file's
//! only job is: open the real vault, supply the real wall-clock timestamp,
//! and print `run_cli`'s result or error to the terminal.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: approve_cli <vault-root> <journal-path> list|approve <id>|reject <id> <reason>");
        std::process::exit(2);
    }
    let vault_root = std::path::PathBuf::from(&args[1]);
    let journal_path = std::path::PathBuf::from(&args[2]);
    let command_args: Vec<String> = args[3..].to_vec();

    let vault = match storage::Vault::open_with_default_indexer(vault_root, &journal_path) {
        Ok(vault) => vault,
        Err(err) => {
            eprintln!("failed to open vault: {err:?}");
            std::process::exit(1);
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    match brain_api::run_cli(&command_args, &vault, &now) {
        Ok(output) => println!("{output}"),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}
