//! The WSL step: `wsl --update` updates the Windows Subsystem for Linux kernel.
//!
//! `wsl --update` shuts down every running WSL distribution and anything
//! running inside it, so the step is opt-in: it only performs the update when
//! `--install-wsl-updates` is given. Without the flag it reports that the
//! update is available but changes nothing. There is no read-only way to tell
//! whether the WSL kernel needs an update, so `check()` always reports the
//! step up to date once `wsl` is detected.
//!
//! The step only runs on Windows, because `wsl` exists only there.

use crate::containers::COLIMA_STOP;
use crate::options::RunOptions;
use crate::output::{flush, human, human_print};
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use std::io;

/// The WSL step.
pub const WSL: StepId = "wsl";

/// WSL only runs on Windows.
const WSL_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the WSL step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    #[cfg(windows)]
    {
        registry.with(WslStep::with_container_stop())
    }
    #[cfg(not(windows))]
    {
        registry.with(WslStep::new())
    }
}

/// The WSL kernel update step.
struct WslStep {
    /// Step that must run before `wsl --update`, so containers are stopped
    /// before WSL shuts down every distribution (Windows only, set when
    /// Podman Desktop or Docker Desktop is the active runtime).
    stop_before: Option<StepId>,
}

impl WslStep {
    /// Creates the WSL step without any container-stop prerequisite.
    #[allow(dead_code)]
    #[must_use]
    fn new() -> Self {
        Self { stop_before: None }
    }

    /// Creates the WSL step that runs after the container runtime is stopped.
    ///
    /// `wsl --update` shuts down every WSL distribution, so container runtimes
    /// that run inside WSL (such as Podman Desktop on Windows) must be stopped
    /// first and started again afterwards.
    #[cfg(windows)]
    #[must_use]
    fn with_container_stop() -> Self {
        Self {
            stop_before: Some(COLIMA_STOP),
        }
    }

    /// Whether the opt-in flag that permits `wsl --update` is set.
    fn update_enabled(options: &RunOptions) -> bool {
        options.install_wsl_updates()
    }
}

impl Step for WslStep {
    fn id(&self) -> StepId {
        WSL
    }

    fn display_name(&self) -> &str {
        "WSL"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        if CommandSpec::new("wsl").args(["--version"]).run().is_ok() {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    /// Reports the step up to date: `wsl` has no read-only query that tells
    /// whether the kernel needs an update, so the step cannot report pending
    /// work without changing anything. The listing is left to `--list-steps`.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    /// Runs `wsl --update` only when the opt-in flag is set. Without it the
    /// step reports that the update is available and changes nothing, because
    /// the update shuts down every running WSL distribution. When the flag is
    /// set the running distributions are listed first and the user must confirm
    /// unless `--yes` was given.
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if !Self::update_enabled(options) {
            human!("wsl: kernel update available; nothing updated (use --install-wsl-updates)");
            return Ok(());
        }

        let running = running_distributions()?;
        if !running.is_empty() {
            human!(
                "warning: wsl --update shuts down every running WSL distribution and anything running inside it:"
            );
            for distro in &running {
                human!("   - {distro}");
            }
            if !options.assume_yes() {
                match confirm_proceed()? {
                    true => {}
                    false => {
                        human!("wsl: update skipped; running WSL distributions would be shut down");
                        return Ok(());
                    }
                }
            }
        }

        human!("wsl: install (WSL kernel update)");
        CommandSpec::new("wsl")
            .args(["--update"])
            .neutral_locale()
            .run()
            .map(|_| ())
            .map_err(|error| StepError::run_from_process(WSL, &error))
    }

    fn platforms(&self) -> &'static [Platform] {
        &WSL_PLATFORMS
    }

    /// Steps that must run before `wsl --update`. Container runtimes that run
    /// inside WSL (e.g. Podman Desktop on Windows) must be stopped first.
    fn prerequisites(&self) -> &'static [StepId] {
        match self.stop_before {
            Some(_) => &[COLIMA_STOP],
            None => &[],
        }
    }
}

/// Returns the distributions that are currently running, as reported by
/// `wsl --list --running`.
fn running_distributions() -> Result<Vec<String>, StepError> {
    let output = CommandSpec::new("wsl")
        .args(["--list", "--running"])
        .neutral_locale()
        .run()
        .map_err(|error| StepError::run_from_process(WSL, &error))?;
    Ok(parse_running_distributions(output.stdout()))
}

/// Parses the output of `wsl --list --running` into distribution names.
///
/// The first line is a header; every other non-empty line lists one
/// distribution. The default distribution is marked with a leading `*`.
fn parse_running_distributions(output: &str) -> Vec<String> {
    let mut distros = Vec::new();
    let mut lines = output.lines();
    if let Some(first) = lines.next() {
        if !first.trim_start().starts_with("NAME") {
            if let Some(name) = distro_name(first) {
                distros.push(name);
            }
        }
    }
    for line in lines {
        if let Some(name) = distro_name(line) {
            distros.push(name);
        }
    }
    distros
}

/// Extracts one distribution name from a line of `wsl --list --running` output.
fn distro_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let line = line.trim_start_matches('*').trim_start();
    let token = line.split_whitespace().next()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}

/// Prints the confirmation prompt and returns `true` when the user answers yes.
fn confirm_proceed() -> Result<bool, StepError> {
    human_print!("Proceed with wsl --update? [y/N]: ");
    flush();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| StepError::run(WSL, format!("failed to read confirmation: {error}")))?;
    Ok(is_confirmation_yes(&answer))
}

/// Returns `true` when the confirmation answer means yes.
fn is_confirmation_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The WSL distribution selected with `--wsl-distro`, when running on Windows.
///
/// On non-Windows platforms this is always `None`, so Linux steps run natively
/// there and the `--wsl-distro` flag has no effect.
#[must_use]
pub fn selected_distro(options: &RunOptions) -> Option<&str> {
    if cfg!(target_os = "windows") {
        options.wsl_distro()
    } else {
        None
    }
}

/// Rewrites `spec` to run `wsl -d <distro> --exec <program> <arguments>` inside
/// the selected WSL distribution on Windows.
///
/// On non-Windows platforms, or when no distribution has been selected, `spec`
/// is returned unchanged so Linux steps keep running natively.
#[must_use]
pub fn inside_command(options: &RunOptions, spec: CommandSpec) -> CommandSpec {
    match selected_distro(options) {
        Some(distro) => wrap_in_distro(distro, &spec),
        None => spec,
    }
}

/// Runs `spec` inside `distro`. `--exec` starts the program directly, without a
/// shell, so no argument needs quoting. The timeout, retry count and
/// neutral-locale settings of `spec` are preserved.
fn wrap_in_distro(distro: &str, spec: &CommandSpec) -> CommandSpec {
    let mut command = CommandSpec::new("wsl")
        .arg("-d")
        .arg(distro)
        .arg("--exec")
        .arg(spec.program());
    for argument in spec.arguments() {
        command = command.arg(argument);
    }
    if let Some(timeout) = spec.timeout_limit() {
        command = command.timeout(timeout);
    }
    command = command.retries(spec.retry_count());
    if spec.uses_neutral_locale() {
        command = command.neutral_locale();
    }
    command
}

#[cfg(test)]
mod tests {
    use super::{
        inside_command, is_confirmation_yes, parse_running_distributions, selected_distro,
        wrap_in_distro, WslStep, WSL,
    };
    use crate::options::RunOptions;
    use crate::process::{CommandSpec, PATH_GUARD};
    #[cfg(not(windows))]
    use crate::step::StepId;
    use crate::step::{Detection, Step};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Creates a stub `wsl` on a temp dir that logs its invocations to
    /// `$FRESHEN_WSL_LOG`. When `installed` is true, `wsl --version` and
    /// `wsl --update` exit 0; otherwise `wsl --version` exits 1 so detection
    /// reports the tool as missing. `running` lists the distributions that
    /// `wsl --list --running` reports. Returns the stub directory; the caller
    /// restores `PATH` and removes it.
    fn stub_wsl(installed: bool, running: &[&str]) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "freshen-wsl-check-{pid}-{id}",
            pid = std::process::id(),
            id = id
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });

        let version_exit = if installed { "0" } else { "1" };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let running_args = running.join(" ");
            let script = format!(
                "#!/bin/sh\n\
                 {{\n\
                   printf '%s' wsl\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 }} >> \"$FRESHEN_WSL_LOG\"\n\
                 case \"$1\" in\n\
                   --version) exit {} ;;\n\
                   --update) exit 0 ;;\n\
                   --list)\n\
                     printf 'NAME            STATE           VERSION\\n'\n\
                     running=\"{}\"\n\
                     for distro in $running; do\n\
                       printf '* %s Running 2\\n' \"$distro\"\n\
                     done\n\
                     exit 0 ;;\n\
                   *) exit 1 ;;\n\
                 esac\n",
                version_exit, running_args
            );
            let stub = dir.join("wsl");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let output_file = dir.join("wsl-output.txt");
            std::fs::write(&output_file, "")
                .unwrap_or_else(|error| panic!("write {}: {error}", output_file.display()));
            let running_echo: String = running
                .iter()
                .map(|distro| format!("echo {} Running 2\r\n", distro))
                .collect();
            let script = format!(
                "@echo off\r\n\
                 >>\"%FRESHEN_WSL_LOG%\" echo wsl %*\r\n\
                 if \"%~1\"==\"--version\" (exit /b {}) else (\n\
                   if \"%~1\"==\"--update\" (exit /b 0) else (\n\
                     if \"%~1\"==\"--list\" (\n\
                       echo NAME            STATE           VERSION\r\n\
                       {}\r\n\
                       exit /b 0\n\
                     ) else (exit /b 1)\n\
                   )\n\
                 )\r\n",
                version_exit, running_echo
            );
            let stub = dir.join("wsl.cmd");
            std::fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        dir
    }

    /// Runs `f` with `wsl` on `PATH`, restoring `PATH` and the log variable
    /// afterwards. Returns `f`'s result together with the recorded calls.
    fn with_wsl_on_path<F, R>(dir: &std::path::Path, f: F) -> (R, Vec<String>)
    where
        F: FnOnce() -> R,
    {
        let log = dir.join("calls.log");
        let _guard = PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let previous_log = std::env::var("FRESHEN_WSL_LOG").ok();
        std::env::set_var("PATH", dir);
        std::env::set_var("FRESHEN_WSL_LOG", &log);

        let result = f();

        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(previous) = previous_log {
            std::env::set_var("FRESHEN_WSL_LOG", previous);
        } else {
            std::env::remove_var("FRESHEN_WSL_LOG");
        }

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let calls: Vec<String> = calls.lines().map(str::trim).map(String::from).collect();
        let _ = std::fs::remove_dir_all(dir);
        (result, calls)
    }

    use crate::platform::Platform;

    #[test]
    fn detect_reports_wsl_installed() {
        let dir = stub_wsl(true, &[]);
        let (result, _calls) = with_wsl_on_path(&dir, || WslStep::new().detect());

        let detection = result.unwrap_or_else(|error| panic!("detection failed: {error}"));
        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn detect_reports_wsl_not_installed() {
        let dir = stub_wsl(false, &[]);
        let (result, _calls) = with_wsl_on_path(&dir, || WslStep::new().detect());

        let detection = result.unwrap_or_else(|error| panic!("detection failed: {error}"));
        assert_eq!(detection, Detection::NotInstalled);
    }

    #[test]
    fn check_is_up_to_date_without_changing_anything() {
        let dir = stub_wsl(true, &[]);
        let (result, calls) = with_wsl_on_path(&dir, || {
            let report = WslStep::new()
                .check()
                .unwrap_or_else(|error| panic!("check failed: {error}"));
            assert!(report.is_up_to_date());
            Ok::<(), crate::step::StepError>(())
        });

        assert!(result.is_ok());
        assert_eq!(calls, Vec::<String>::new());
    }

    #[test]
    fn run_without_the_opt_in_does_not_update() {
        let dir = stub_wsl(true, &[]);
        let (result, calls) = with_wsl_on_path(&dir, || WslStep::new().run());

        assert!(result.is_ok());
        assert_eq!(calls, Vec::<String>::new(), "calls: {calls:?}");
    }

    #[test]
    fn run_with_the_opt_in_runs_wsl_update() {
        let dir = stub_wsl(true, &[]);
        let options = RunOptions::new().with_install_wsl_updates(true);
        let (result, calls) = with_wsl_on_path(&dir, || WslStep::new().run_with_options(&options));

        assert!(result.is_ok(), "wsl --update should succeed");
        assert_eq!(
            calls,
            vec!["wsl --list --running", "wsl --update"],
            "calls: {calls:?}"
        );
    }

    #[test]
    fn run_with_the_opt_in_and_running_distributions_proceeds_with_yes() {
        let dir = stub_wsl(true, &["Ubuntu"]);
        let options = RunOptions::new()
            .with_install_wsl_updates(true)
            .with_assume_yes(true);
        let (result, calls) = with_wsl_on_path(&dir, || WslStep::new().run_with_options(&options));

        assert!(result.is_ok(), "wsl --update should succeed with --yes");
        assert_eq!(
            calls,
            vec!["wsl --list --running", "wsl --update"],
            "calls: {calls:?}"
        );
    }

    #[test]
    fn parse_running_distributions_parses_the_listed_distributions() {
        let output = "NAME            STATE           VERSION\n\
                      * Ubuntu        Running         2\n\
                      docker-desktop  Running         2\n";
        let distros = parse_running_distributions(output);
        assert_eq!(distros, ["Ubuntu", "docker-desktop"]);
    }

    #[test]
    fn parse_running_distributions_ignores_the_header_and_empty_lines() {
        let output = "NAME            STATE           VERSION\n\nUbuntu   Running   2\n";
        let distros = parse_running_distributions(output);
        assert_eq!(distros, ["Ubuntu"]);
    }

    #[test]
    fn parse_running_distributions_without_a_header() {
        let output = "Ubuntu\n";
        let distros = parse_running_distributions(output);
        assert_eq!(distros, ["Ubuntu"]);
    }

    #[test]
    fn parse_running_distributions_with_no_running_distributions_is_empty() {
        let output = "NAME            STATE           VERSION\n";
        let distros = parse_running_distributions(output);
        assert!(distros.is_empty());
    }

    #[test]
    fn is_confirmation_yes_accepts_y_and_yes() {
        assert!(is_confirmation_yes("y"));
        assert!(is_confirmation_yes("yes"));
        assert!(is_confirmation_yes("Y"));
        assert!(is_confirmation_yes("YES"));
        assert!(!is_confirmation_yes(""));
        assert!(!is_confirmation_yes("n"));
        assert!(!is_confirmation_yes("no"));
        assert!(!is_confirmation_yes("maybe"));
    }

    #[test]
    fn the_step_only_runs_on_windows() {
        let registry = super::register(crate::registry::Registry::new());

        assert_eq!(registry.validate(), Ok(()));
        let step = registry
            .steps()
            .find(|step| step.id() == WSL)
            .unwrap_or_else(|| panic!("{WSL} is not registered"));
        assert_eq!(step.platforms(), [Platform::Windows]);
    }

    #[test]
    fn the_step_is_not_part_of_a_run_on_other_platforms() {
        let registry = super::register(crate::registry::Registry::new());

        for platform in [
            crate::platform::Platform::Linux,
            crate::platform::Platform::Macos,
        ] {
            let ids: Vec<&str> = registry
                .ordered_for(platform)
                .unwrap_or_else(|error| panic!("registry order failed: {error}"))
                .iter()
                .map(|step| step.id())
                .collect();
            assert!(!ids.contains(&WSL), "{platform}");
        }
    }

    /// `selected_distro` reports no distribution on non-Windows platforms, so
    /// Linux steps run natively there even when `--wsl-distro` is given.
    #[cfg(not(windows))]
    #[test]
    fn selected_distro_returns_none_on_non_windows_platforms() {
        let options = RunOptions::new().with_wsl_distro(Some("Ubuntu".to_owned()));

        assert_eq!(selected_distro(&options), None);
        assert_eq!(selected_distro(&RunOptions::new()), None);
    }

    /// `inside_command` returns the spec unchanged on non-Windows platforms so
    /// Linux steps keep running natively there.
    #[cfg(not(windows))]
    #[test]
    fn inside_command_returns_spec_unchanged_on_non_windows_platforms() {
        let options = RunOptions::new().with_wsl_distro(Some("Ubuntu".to_owned()));
        let spec = CommandSpec::new("apt-get")
            .args(["update"])
            .neutral_locale()
            .retries(2)
            .timeout(std::time::Duration::from_secs(60));

        let rewritten = inside_command(&options, spec.clone());

        assert_eq!(rewritten, spec);
    }

    #[cfg(windows)]
    #[test]
    fn selected_distro_returns_the_distro_on_windows() {
        let options = RunOptions::new().with_wsl_distro(Some("Ubuntu".to_owned()));

        assert_eq!(selected_distro(&options), Some("Ubuntu"));
        assert_eq!(selected_distro(&RunOptions::new()), None);
    }

    #[cfg(windows)]
    #[test]
    fn inside_command_runs_the_program_in_the_distro_on_windows() {
        let options = RunOptions::new().with_wsl_distro(Some("Ubuntu".to_owned()));
        let spec = CommandSpec::new("apt-get").args(["update"]);

        let rewritten = inside_command(&options, spec);

        assert_eq!(rewritten.program(), "wsl");
    }

    /// The wrapping itself does not depend on the platform, so every OS tests it.
    #[test]
    fn a_command_is_wrapped_to_run_inside_the_distro() {
        let spec = CommandSpec::new("apt-get")
            .args(["update"])
            .neutral_locale()
            .retries(2)
            .timeout(std::time::Duration::from_secs(60));

        let wrapped = wrap_in_distro("Ubuntu", &spec);

        assert_eq!(wrapped.program(), "wsl");
        let arguments: Vec<String> = wrapped
            .arguments()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(arguments, ["-d", "Ubuntu", "--exec", "apt-get", "update"]);
        assert_eq!(
            wrapped.timeout_limit(),
            Some(std::time::Duration::from_secs(60))
        );
        assert_eq!(wrapped.retry_count(), 2);
        assert!(wrapped.uses_neutral_locale());
    }

    /// `wsl --update` must run after the container runtime is stopped, so
    /// containers running inside WSL are cleanly shut down first.
    #[cfg(windows)]
    #[test]
    fn wsl_step_requires_container_stop_before_update() {
        use crate::containers::COLIMA_STOP;

        let step = WslStep::with_container_stop();
        assert_eq!(step.prerequisites(), &[COLIMA_STOP]);
    }

    /// On non-Windows platforms, `wsl --update` has no container-stop
    /// prerequisite because no WSL-backed runtime runs there.
    #[cfg(not(windows))]
    #[test]
    fn wsl_step_has_no_container_stop_prerequisite_on_non_windows() {
        let step = WslStep::new();
        assert_eq!(step.prerequisites(), &[] as &[StepId]);
    }
}
