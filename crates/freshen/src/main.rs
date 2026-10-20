//! Thin CLI wrapper around the `freshen-lib` crate: all logic lives in the
//! library, this binary only parses arguments and prints output.

use std::process::ExitCode;

use clap::Parser;
use freshen_lib::{Platform, Registry, Runner};

mod cli;

use cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = cli.run_options();
    let registry = Registry::new();

    let Some(platform) = Platform::current() else {
        eprintln!("freshen: unsupported operating system");
        return ExitCode::FAILURE;
    };

    let steps = match options.plan(&registry, platform) {
        Ok(steps) => steps,
        Err(error) => {
            eprintln!("freshen: {error}");
            return ExitCode::FAILURE;
        }
    };

    let summary = Runner::with_options(steps, options).run();
    print!("{summary}");
    summary.exit_code()
}
