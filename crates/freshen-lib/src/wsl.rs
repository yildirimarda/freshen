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

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// The WSL step.
pub const WSL: StepId = "wsl";

/// WSL only runs on Windows.
const WSL_PLATFORMS: [Platform; 1] = [Platform::Windows];

/// Registers the WSL step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(WslStep)
}

/// The WSL kernel update step.
struct WslStep;

impl WslStep {
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
    /// the update shuts down every running WSL distribution.
    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if Self::update_enabled(options) {
            human!("wsl: install (WSL kernel update)");
            CommandSpec::new("wsl")
                .args(["--update"])
                .neutral_locale()
                .run()
                .map(|_| ())
                .map_err(|error| StepError::run_from_process(WSL, &error))
        } else {
            human!("wsl: kernel update available; nothing updated (use --install-wsl-updates)");
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &WSL_PLATFORMS
    }
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
    use super::{inside_command, selected_distro, wrap_in_distro, WslStep, WSL};
    use crate::options::RunOptions;
    use crate::process::{CommandSpec, PATH_GUARD};
    use crate::step::{Detection, Step};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Creates a stub `wsl` on a temp dir that logs its invocations to
    /// `$FRESHEN_WSL_LOG`. When `installed` is true, `wsl --version` and
    /// `wsl --update` exit 0; otherwise `wsl --version` exits 1 so detection
    /// reports the tool as missing. Returns the stub directory; the caller
    /// restores `PATH` and removes it.
    fn stub_wsl(installed: bool) -> std::path::PathBuf {
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let version_exit = if installed { "0" } else { "1" };
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
                   *) exit 1 ;;\n\
                 esac\n",
                version_exit
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
            let version_exit = if installed { "0" } else { "1" };
            let script = format!(
                "@echo off\r\n\
                 >>\"%FRESHEN_WSL_LOG%\" echo wsl %*\r\n\
                 if \"%~1\"==\"--version\" (exit /b {}) else (\n\
                   if \"%~1\"==\"--update\" (exit /b 0) else (exit /b 1)\n\
                 )\r\n",
                version_exit
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
        let dir = stub_wsl(true);
        let (result, _calls) = with_wsl_on_path(&dir, || WslStep.detect());

        let detection = result.unwrap_or_else(|error| panic!("detection failed: {error}"));
        assert_eq!(detection, Detection::Installed);
    }

    #[test]
    fn detect_reports_wsl_not_installed() {
        let dir = stub_wsl(false);
        let (result, _calls) = with_wsl_on_path(&dir, || WslStep.detect());

        let detection = result.unwrap_or_else(|error| panic!("detection failed: {error}"));
        assert_eq!(detection, Detection::NotInstalled);
    }

    #[test]
    fn check_is_up_to_date_without_changing_anything() {
        let dir = stub_wsl(true);
        let (result, calls) = with_wsl_on_path(&dir, || {
            let report = WslStep
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
        let dir = stub_wsl(true);
        let (result, calls) = with_wsl_on_path(&dir, || WslStep.run());

        assert!(result.is_ok());
        assert_eq!(calls, Vec::<String>::new(), "calls: {calls:?}");
    }

    #[test]
    fn run_with_the_opt_in_runs_wsl_update() {
        let dir = stub_wsl(true);
        let options = RunOptions::new().with_install_wsl_updates(true);
        let (result, calls) = with_wsl_on_path(&dir, || WslStep.run_with_options(&options));

        assert!(result.is_ok(), "wsl --update should succeed");
        assert_eq!(calls, vec!["wsl --update"], "calls: {calls:?}");
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
}
