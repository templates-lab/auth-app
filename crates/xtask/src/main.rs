//! Workspace automation entry point (the `cargo xtask` pattern).
//!
//! Keeping dev/CI chores here — rather than in shell scripts scattered across
//! the repo — gives the monorepo a single, discoverable task runner. Real tasks
//! (fmt/lint/test/ci) are wired up alongside the backend crates; for now this
//! binary just documents the available surface and gives the Cargo workspace a
//! buildable member.

use std::process::ExitCode;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("help") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("xtask: unknown task `{other}`\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "\
Usage: cargo xtask <task>

Tasks:
    help    Show this message

More tasks (fmt, lint, test, ci) are added as the backend crates land."
    );
}
