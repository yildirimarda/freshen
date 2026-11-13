//! Thin CLI wrapper around the `freshen-lib` crate: all logic lives in the
//! library, this binary only parses arguments and prints output.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use freshen_lib::doctor;
use freshen_lib::logging::{LogFile, LoggingEnvironment};
use freshen_lib::sudo::{HostSudoProbe, SudoState};
#[cfg(windows)]
use freshen_lib::HostElevationProbe;
use freshen_lib::{
    detect_active_runtime, exit, install_interrupt_handler, list_steps, wait_for_package_managers,
    ActiveContainerRuntime, AppLists, Config, ConfigEnvironment, ContainerRuntimeKind,
    CustomStepConfig, CustomStepError, HostContainerRuntimeProbe, HostPackageManagerProbe,
    HostPreflightProbe, NoElevationProbe, PackageManagerConfig, PackageManagerProbe, Platform,
    PreflightReport, Registry, RunLock, Runner, SystemInfo, WaitOutcome,
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

    // Log to a rotating file in the OS-appropriate location. Logging never
    // fails a run: when the log cannot be opened it is simply skipped.
    let mut logger = LogFile::open(LoggingEnvironment::from_env().log_path(platform)).ok();
    log_event(
        &mut logger,
        &format!("freshen {} started", freshen_lib::version()),
    );

    let config_path = cli
        .config
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| ConfigEnvironment::from_env().config_path(platform));
    let (config, config_error) = match load_config(&cli, platform) {
        Ok(config) => (config, None),
        Err(error) => {
            if cli.doctor {
                (Config::default(), Some(error))
            } else {
                log_event(&mut logger, &format!("freshen: {error}"));
                eprintln!("freshen: {error}");
                return exit::code(exit::STEP_FAILURE);
            }
        }
    };
    let options = match cli.run_options_with(&config) {
        Ok(options) => options,
        Err(error) => {
            log_event(&mut logger, &format!("freshen: {error}"));
            eprintln!("freshen: {error}");
            return exit::code(exit::STEP_FAILURE);
        }
    };
    let app_lists = config.app_lists().merged_with(AppLists::from_env());
    let options = options
        .with_app_lists(app_lists)
        .with_container_runtime(active_container_runtime(cli.container_runtime))
        .with_pre_commit_repositories(config.pre_commit_repositories());

    // Validate `sudo` credentials once up front so every step that needs root
    // can be skipped with a "needs sudo" reason instead of failing mid-run.
    let privilege = SystemInfo::detect().privilege();
    let sudo_available = SudoState::from_privilege_and_probe(privilege, &HostSudoProbe).available();

    let registry = match build_registry(
        &config.custom_steps,
        options
            .container_runtime()
            .map(ActiveContainerRuntime::kind),
    ) {
        Ok(registry) => registry,
        Err(error) => {
            log_event(&mut logger, &format!("freshen: {error}"));
            eprintln!("freshen: {error}");
            return exit::code(exit::STEP_FAILURE);
        }
    };

    let steps = match options.plan(&registry, platform) {
        Ok(steps) => steps,
        Err(error) => {
            log_event(&mut logger, &format!("freshen: {error}"));
            eprintln!("freshen: {error}");
            return exit::code(exit::STEP_FAILURE);
        }
    };

    if cli.list_steps {
        log_event(&mut logger, "listed steps");
        print!("{}", list_steps(&registry, &options, platform));
        return exit::code(exit::OK);
    }

    if cli.doctor {
        let report = doctor::doctor(
            &registry,
            &options,
            platform,
            &config_path,
            config_error.as_ref(),
            privilege,
            &HostPreflightProbe,
        );
        log_event(
            &mut logger,
            &format!("doctor completed with exit code {:?}", report.exit_code()),
        );
        print!("{report}");
        return report.exit_code();
    }

    if cli.check {
        let summary = Runner::with_options_and_sudo(
            steps,
            options,
            sudo_available,
            privilege,
            &NoElevationProbe,
        )
        .check();
        log_event(
            &mut logger,
            &format!("check completed with exit code {:?}", summary.exit_code()),
        );
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
                log_event(&mut logger, &format!("freshen: {error}"));
                eprintln!("freshen: {error}");
                return exit::code(exit::STEP_FAILURE);
            }
        }
    };

    if !options.is_dry_run() && !preflight_skipped() {
        let report = run_preflight(&options);
        if !report.is_ok() {
            log_event(&mut logger, "freshen: preflight check failed");
            eprint!("{report}");
            return exit::code(exit::PREFLIGHT_FAILED);
        }

        if options.package_manager().is_enabled() {
            if let Some(code) = wait_for_package_managers_or_abort(options.package_manager()) {
                log_event(
                    &mut logger,
                    &format!(
                        "freshen: package manager check failed with exit code {:?}",
                        code
                    ),
                );
                return code;
            }
        }
    }

    let summary =
        Runner::with_options_and_sudo(steps, options, sudo_available, privilege, elevation_probe())
            .run();
    log_event(
        &mut logger,
        &format!("run completed with exit code {:?}", summary.exit_code()),
    );
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
    let mut config = if let Some(path) = &cli.config {
        Config::load(path)?
    } else {
        let path = ConfigEnvironment::from_env().config_path(platform);
        Config::load_optional(&path)?.unwrap_or_default()
    };
    config.apply_env_overrides(std::env::vars_os().map(|(key, value)| {
        (
            key.to_string_lossy().into_owned(),
            value.to_string_lossy().into_owned(),
        )
    }));
    Ok(config)
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

fn build_registry(
    custom_steps: &[CustomStepConfig],
    runtime: Option<ContainerRuntimeKind>,
) -> Result<Registry, CustomStepError> {
    #[cfg(feature = "test-harness")]
    if test_steps::requested() {
        return Ok(test_steps::with_probe_steps(Registry::new()));
    }

    let mut registry = Registry::new();
    registry = freshen_lib::containers::register_for(registry, runtime);
    registry = freshen_lib::homebrew::register(registry);
    registry = freshen_lib::antidote::register(registry);
    registry = freshen_lib::aws::register(registry);
    registry = freshen_lib::az::register(registry);
    registry = freshen_lib::aur::register(registry);
    registry = freshen_lib::emerge::register(registry);
    registry = freshen_lib::apk::register(registry);
    registry = freshen_lib::zypper::register(registry);
    registry = freshen_lib::pacman::register(registry);
    registry = freshen_lib::dnf::register(registry);
    registry = freshen_lib::apt::register(registry);
    registry = freshen_lib::termux::register(registry);
    registry = freshen_lib::rpm_ostree::register(registry);
    registry = freshen_lib::bootc::register(registry);
    registry = freshen_lib::flatpak::register(registry);
    registry = freshen_lib::fisher::register(registry);
    registry = freshen_lib::firmware::register(registry);
    registry = freshen_lib::gcloud::register(registry);
    registry = freshen_lib::snap::register(registry);
    registry = freshen_lib::nix::register(registry);
    registry = freshen_lib::tooling::register(registry);
    registry = freshen_lib::elixir::register(registry);
    registry = freshen_lib::composer::register(registry);
    registry = freshen_lib::conda::register(registry);
    registry = freshen_lib::mamba::register(registry);
    registry = freshen_lib::nvm::register(registry);
    registry = freshen_lib::nvim::register(registry);
    registry = freshen_lib::omz::register(registry);
    registry = freshen_lib::zinit::register(registry);
    registry = freshen_lib::tpm::register(registry);
    registry = freshen_lib::packer::register(registry);
    registry = freshen_lib::mason::register(registry);
    registry = freshen_lib::treesitter::register(registry);
    registry = freshen_lib::fnm::register(registry);
    registry = freshen_lib::volta::register(registry);
    registry = freshen_lib::pip::register(registry);
    registry = freshen_lib::pipx::register(registry);
    registry = freshen_lib::rustup::register(registry);
    registry = freshen_lib::sdkman::register(registry);
    registry = freshen_lib::dotnet_tools::register(registry);
    registry = freshen_lib::jenv::register(registry);
    registry = freshen_lib::tfenv::register(registry);
    registry = freshen_lib::asdf::register(registry);
    registry = freshen_lib::cargo_update::register(registry);
    registry = freshen_lib::pnpm::register(registry);
    registry = freshen_lib::yarn::register(registry);
    registry = freshen_lib::bun::register(registry);
    registry = freshen_lib::pyenv::register(registry);
    registry = freshen_lib::rbenv::register(registry);
    registry = freshen_lib::gem::register(registry);
    registry = freshen_lib::go::register(registry);
    registry = freshen_lib::appstore::register(registry);
    registry = freshen_lib::vscode::register(registry);
    registry = freshen_lib::cursor::register(registry);
    registry = freshen_lib::vscodium::register(registry);
    registry = freshen_lib::gh::register(registry);
    registry = freshen_lib::git_lfs::register(registry);
    registry = freshen_lib::pre_commit::register(registry);
    registry = freshen_lib::helm::register(registry);
    registry = freshen_lib::kubectl::register(registry);
    registry = freshen_lib::winget::register(registry);
    registry = freshen_lib::chocolatey::register(registry);
    registry = freshen_lib::microsoft_store::register(registry);
    registry = freshen_lib::scoop::register(registry);
    registry = freshen_lib::wsl::register(registry);
    registry = freshen_lib::windows_update::register(registry);

    // Custom steps run after the built-in steps, ordered by their `order`
    // field. Custom steps that share an order keep their configuration order,
    // and a step that omits `order` runs after every ordered custom step.
    let mut configured: Vec<(usize, freshen_lib::custom_step::CustomStep)> =
        Vec::with_capacity(custom_steps.len());
    for config in custom_steps {
        let step = freshen_lib::custom_step::CustomStep::try_from_config(config)?;
        configured.push((step.order(), step));
    }
    configured.sort_by_key(|(order, _)| *order);

    for (_, step) in configured {
        registry.register(step);
    }

    Ok(registry)
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

/// Returns the elevation probe used when running (not `--check`) steps.
///
/// On Windows this is the real `HostElevationProbe` which shows a single UAC
/// prompt; on other platforms elevation is unavailable, so steps that need
/// administrator rights are skipped instead.
fn elevation_probe() -> &'static dyn freshen_lib::ElevationProbe {
    #[cfg(windows)]
    {
        &HostElevationProbe
    }
    #[cfg(not(windows))]
    {
        &NoElevationProbe
    }
}

/// Writes one line to the rotating log file, when one is open.
///
/// Logging is best effort: a run must never fail because its log could not be
/// written.
fn log_event(logger: &mut Option<LogFile>, message: &str) {
    if let Some(logger) = logger {
        let _ = logger.write(message);
    }
}
