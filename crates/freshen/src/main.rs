//! Thin CLI wrapper around the `freshen-lib` crate: all logic lives in the
//! library, this binary only parses arguments and prints output.

use std::process::ExitCode;

use clap::Parser;
use freshen_lib::{list_steps, Platform, Registry, Runner};

mod cli;

#[cfg(feature = "test-harness")]
mod test_steps;

use cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = cli.run_options();
    let registry = build_registry();

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

    if cli.list_steps {
        print!("{}", list_steps(&registry, options.selection(), platform));
        return ExitCode::SUCCESS;
    }

    let summary = Runner::with_options(steps, options).run();
    print!("{summary}");
    summary.exit_code()
}

fn build_registry() -> Registry {
    #[cfg(feature = "test-harness")]
    if test_steps::requested() {
        return test_steps::with_probe_steps(Registry::new());
    }

    freshen_lib::tooling::register(freshen_lib::homebrew::register(
        freshen_lib::containers::register(Registry::new()),
    ))
}
