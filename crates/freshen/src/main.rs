//! Thin CLI wrapper around the `freshen-lib` crate: all logic lives in the
//! library, this binary only parses arguments and prints output.

use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("freshen {}", freshen_lib::version());
        return ExitCode::SUCCESS;
    }
    println!(
        "freshen {} (library scaffold; steps land in PLAN.md Milestone 2)",
        freshen_lib::version()
    );
    ExitCode::SUCCESS
}
