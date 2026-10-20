//! Thin CLI wrapper around the `freshen-lib` crate: all logic lives in the
//! library, this binary only parses arguments and prints output.

use std::process::ExitCode;

use clap::Parser;
use freshen_lib::{list_steps, HostPreflightProbe, Platform, PreflightReport, Registry, Runner};

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

    if cli.check {
        let summary = Runner::with_options(steps, options).check();
        if cli.json {
            println!("{}", summary.to_json());
        } else if cli.markdown {
            print!("{}", summary.to_markdown());
        } else {
            print!("{summary}");
        }
        return summary.exit_code();
    }

    if !options.is_dry_run() && !preflight_skipped() {
        let report = run_preflight(&options);
        if !report.is_ok() {
            eprint!("{report}");
            return ExitCode::from(3);
        }
    }

    let summary = Runner::with_options(steps, options).run();
    if cli.json {
        println!("{}", summary.to_json());
    } else if cli.markdown {
        print!("{}", summary.to_markdown());
    } else {
        print!("{summary}");
    }
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

fn run_preflight(options: &freshen_lib::RunOptions) -> PreflightReport {
    #[cfg(feature = "test-harness")]
    if let Some(probe) = test_steps::preflight_probe() {
        return PreflightReport::run(&probe, options.preflight());
    }

    PreflightReport::run(&HostPreflightProbe, options.preflight())
}

fn preflight_skipped() -> bool {
    std::env::var(freshen_lib::preflight::SKIP_ENV)
        .ok()
        .is_some_and(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && value != "0" && value != "false" && value != "no"
        })
}
