//! Thin CLI wrapper around the `freshen-lib` crate: all logic lives in the
//! library, this binary only parses arguments and prints output.

use std::process::ExitCode;

use clap::Parser;
use freshen_lib::{
    detect_active_runtime, exit, install_interrupt_handler, list_steps, wait_for_package_managers,
    ActiveContainerRuntime, AppLists, Config, ConfigEnvironment, ContainerRuntimeKind,
    HostContainerRuntimeProbe, HostPackageManagerProbe, HostPreflightProbe, PackageManagerConfig,
    PackageManagerProbe, Platform, PreflightReport, Registry, RunLock, Runner, WaitOutcome,
};

mod cli;

#[cfg(feature = "test-harness")]
mod test_steps;

use cli::Cli;

fn main() -> ExitCode {
    if let Err(error) = install_interrupt_handler() {
        eprintln!("freshen: {error}");
        return exit::code(exit::STEP_FAILURE);
    }

    let cli = Cli::parse();
    if cli.json || cli.markdown {
        freshen_lib::route_to_stderr();
    }

    let Some(platform) = Platform::current() else {
        eprintln!("freshen: unsupported operating system");
        return exit::code(exit::STEP_FAILURE);
    };

    let config = match load_config(&cli, platform) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("freshen: {error}");
            return exit::code(exit::STEP_FAILURE);
        }
    };
    let options = cli.run_options_with(&config);
    let app_lists = config.app_lists().merged_with(AppLists::from_env());
    let options = options
        .with_app_lists(app_lists)
        .with_container_runtime(active_container_runtime(cli.container_runtime));
    let registry = build_registry(
        options
            .container_runtime()
            .map(ActiveContainerRuntime::kind),
    );

    let steps = match options.plan(&registry, platform) {
        Ok(steps) => steps,
        Err(error) => {
            eprintln!("freshen: {error}");
            return exit::code(exit::STEP_FAILURE);
        }
    };

    if cli.list_steps {
        print!("{}", list_steps(&registry, &options, platform));
        return exit::code(exit::OK);
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

    let _lock = if options.is_dry_run() {
        None
    } else {
        match RunLock::acquire() {
            Ok(lock) => Some(lock),
            Err(error) => {
                eprintln!("freshen: {error}");
                return exit::code(exit::STEP_FAILURE);
            }
        }
    };

    if !options.is_dry_run() && !preflight_skipped() {
        let report = run_preflight(&options);
        if !report.is_ok() {
            eprint!("{report}");
            return exit::code(exit::PREFLIGHT_FAILED);
        }

        if options.package_manager().is_enabled() {
            if let Some(code) = wait_for_package_managers_or_abort(options.package_manager()) {
                return code;
            }
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

/// Loads the configuration file.
///
/// A file named with `--config` has to exist; the file in the default
/// location is optional.
fn load_config(cli: &Cli, platform: Platform) -> Result<Config, freshen_lib::ConfigError> {
    if let Some(path) = &cli.config {
        return Config::load(path);
    }

    let path = ConfigEnvironment::from_env().config_path(platform);
    Ok(Config::load_optional(&path)?.unwrap_or_default())
}

/// Resolves the runtime from `--container-runtime`, falling back to the
/// runtime detected on the host.
fn active_container_runtime(
    overridden: Option<ContainerRuntimeKind>,
) -> Option<ActiveContainerRuntime> {
    overridden
        .map(ActiveContainerRuntime::overridden)
        .or_else(|| detect_active_runtime(&HostContainerRuntimeProbe))
}

fn build_registry(runtime: Option<ContainerRuntimeKind>) -> Registry {
    #[cfg(feature = "test-harness")]
    if test_steps::requested() {
        return test_steps::with_probe_steps(Registry::new());
    }

    let mut registry = Registry::new();
    registry = freshen_lib::containers::register_for(registry, runtime);
    registry = freshen_lib::homebrew::register(registry);
    registry = freshen_lib::aur::register(registry);
    registry = freshen_lib::emerge::register(registry);
    registry = freshen_lib::apk::register(registry);
    registry = freshen_lib::zypper::register(registry);
    registry = freshen_lib::pacman::register(registry);
    registry = freshen_lib::dnf::register(registry);
    registry = freshen_lib::apt::register(registry);
    registry = freshen_lib::tooling::register(registry);
    registry = freshen_lib::appstore::register(registry);
    registry
}

fn run_preflight(options: &freshen_lib::RunOptions) -> PreflightReport {
    #[cfg(feature = "test-harness")]
    if let Some(probe) = test_steps::preflight_probe() {
        return PreflightReport::run(&probe, options.preflight());
    }

    PreflightReport::run(&HostPreflightProbe, options.preflight())
}

fn package_manager_probe() -> Box<dyn PackageManagerProbe> {
    #[cfg(feature = "test-harness")]
    if let Some(probe) = test_steps::package_manager_probe() {
        return Box::new(probe);
    }

    Box::new(HostPackageManagerProbe)
}

fn wait_for_package_managers_or_abort(config: &PackageManagerConfig) -> Option<ExitCode> {
    let probe = package_manager_probe();
    let outcome = wait_for_package_managers(probe.as_ref(), config);

    let (waited, processes, interrupted) = match outcome {
        WaitOutcome::Idle | WaitOutcome::Waited { .. } => return None,
        WaitOutcome::Busy { waited, processes } => (waited, processes, false),
        WaitOutcome::Interrupted { waited, processes } => (waited, processes, true),
    };

    let processes = processes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if interrupted {
        eprintln!("freshen: interrupted while waiting for another package manager: {processes}");
        return Some(exit::code(exit::INTERRUPTED));
    }
    if waited.is_zero() {
        eprintln!("freshen: another package manager is running: {processes}");
    } else {
        eprintln!(
            "freshen: another package manager is still running after waiting {:.1}s: {processes}",
            waited.as_secs_f64()
        );
    }

    Some(exit::code(exit::PREFLIGHT_FAILED))
}

fn preflight_skipped() -> bool {
    std::env::var(freshen_lib::preflight::SKIP_ENV)
        .ok()
        .is_some_and(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && value != "0" && value != "false" && value != "no"
        })
}
