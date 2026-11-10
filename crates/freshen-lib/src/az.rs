//! The Azure CLI step: `az upgrade --yes` upgrades the Azure CLI when it is
//! installed outside a system package manager.
//!
//! `az` is the command line tool of the Azure CLI. The step runs `az upgrade
//! --yes` whenever `az` is available on `PATH` and is not owned by a system
//! package manager (Homebrew, apt/dpkg, dnf, pacman, winget, Chocolatey or
//! Scoop). Those managers keep their own Azure CLI packages current, so
//! freshen leaves them alone. It is skipped entirely when `az` is absent, so
//! it is safe to register on every machine.
//!
//! The Azure CLI manages user-level components, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, Operation, Step, StepError, StepId};
use std::path::Path;
use std::sync::Arc;

/// The Azure CLI step.
pub const AZ: StepId = "az";

/// The Azure CLI runs on every platform freshen supports.
const AZ_PLATFORMS: [Platform; 3] = [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the Azure CLI step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(AzStep::new())
}

/// One invocation of `az` and everything it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzOutput {
    success: bool,
    text: String,
}

impl AzOutput {
    /// A successful `az` invocation.
    #[must_use]
    fn success(text: impl Into<String>) -> Self {
        Self {
            success: true,
            text: text.into(),
        }
    }

    /// A failed `az` invocation.
    #[must_use]
    fn failure(text: impl Into<String>) -> Self {
        Self {
            success: false,
            text: text.into(),
        }
    }

    /// Whether the command finished successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Everything the command printed, on stdout and on stderr.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The `az` commands the Azure CLI step needs.
///
/// The trait exists so the step can be tested against recorded transcripts
/// without the real `az` binary.
pub trait AzClient: Send + Sync {
    /// The output of `az upgrade --yes`.
    fn upgrade(&self) -> AzOutput;
}

/// The real `az` command line.
struct AzCommandClient;

impl AzClient for AzCommandClient {
    fn upgrade(&self) -> AzOutput {
        run(CommandSpec::new("az")
            .args(["upgrade", "--yes"])
            .neutral_locale())
    }
}

/// Runs an `az` command and collects everything it printed.
fn run(command: CommandSpec) -> AzOutput {
    match command.run() {
        Ok(output) => AzOutput::success(combined(output.stdout(), output.stderr())),
        Err(error) => {
            let printed = error
                .output()
                .map(|output| combined(output.stdout(), output.stderr()))
                .unwrap_or_default();
            if printed.trim().is_empty() {
                AzOutput::failure(error.to_string())
            } else {
                AzOutput::failure(printed)
            }
        }
    }
}

/// Joins what a command printed on stdout and on stderr.
fn combined(stdout: &str, stderr: &str) -> String {
    let mut text = stdout.to_owned();
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(stderr);
    }
    text
}

/// The Azure CLI step.
struct AzStep {
    client: Arc<dyn AzClient>,
}

impl AzStep {
    fn new() -> Self {
        Self {
            client: Arc::new(AzCommandClient),
        }
    }

    #[cfg(test)]
    fn with_client(client: Arc<dyn AzClient>) -> Self {
        Self { client }
    }

    /// Upgrades the Azure CLI.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("az: upgrade --yes");
        let output = self.client.upgrade();
        if !output.is_success() {
            failures.push(StepError::new(
                AZ,
                Operation::Run,
                format!("`az upgrade --yes` failed: {}", output.text().trim()),
            ));
        }
    }
}

impl Step for AzStep {
    fn id(&self) -> StepId {
        AZ
    }

    fn display_name(&self) -> &str {
        "az"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_az())
    }

    /// `az` has no read-only "what is outdated" query, so report up to date.
    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn platforms(&self) -> &'static [Platform] {
        &AZ_PLATFORMS
    }

    /// The Azure CLI manages user-level components, so it must never run as
    /// root.
    fn is_user_level(&self) -> bool {
        true
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `az` is installed outside a system package manager.
fn detect_az() -> Detection {
    let Some(az_path) = crate::executable::resolve("az") else {
        return Detection::NotInstalled;
    };
    if az_installed_with_system_package_manager(&az_path) {
        return Detection::NotInstalled;
    }
    Detection::Installed
}

/// Whether `az` is owned by a system package manager, in which case that
/// manager's own upgrade keeps the Azure CLI current and `az upgrade` is
/// skipped so the two do not fight over the installation.
fn az_installed_with_system_package_manager(az_path: &Path) -> bool {
    az_installed_with_brew(az_path)
        || az_installed_with_dpkg()
        || az_installed_with_dnf()
        || az_installed_with_pacman()
        || az_installed_with_winget()
        || az_installed_with_choco()
        || az_installed_with_scoop()
}

/// Whether Homebrew owns the Azure CLI installation (the `az` binary lives
/// inside a Homebrew prefix).
fn az_installed_with_brew(az_path: &Path) -> bool {
    crate::homebrew::brew_installs()
        .iter()
        .any(|install| az_path.starts_with(&install.prefix))
}

/// Whether dpkg (and therefore apt on Debian/Ubuntu) owns the Azure CLI
/// installation (`dpkg -s azure-cli`).
fn az_installed_with_dpkg() -> bool {
    command_exists("dpkg")
        && CommandSpec::new("dpkg")
            .args(["-s", "azure-cli"])
            .run()
            .is_ok()
}

/// Whether dnf owns the Azure CLI installation.
fn az_installed_with_dnf() -> bool {
    command_exists("dnf")
        && CommandSpec::new("dnf")
            .args(["list", "installed", "azure-cli"])
            .run()
            .is_ok()
}

/// Whether pacman owns the Azure CLI installation.
fn az_installed_with_pacman() -> bool {
    command_exists("pacman")
        && CommandSpec::new("pacman")
            .args(["-Q", "azure-cli"])
            .run()
            .is_ok()
}

/// Whether winget owns the Azure CLI installation.
fn az_installed_with_winget() -> bool {
    command_exists("winget")
        && CommandSpec::new("winget")
            .args(["list", "Microsoft.AzureCLI"])
            .run()
            .is_ok()
}

/// Whether Chocolatey owns the Azure CLI installation.
fn az_installed_with_choco() -> bool {
    command_exists("choco")
        && CommandSpec::new("choco")
            .args(["list", "--local-only", "azure-cli"])
            .run()
            .is_ok()
}

/// Whether Scoop owns the Azure CLI installation.
fn az_installed_with_scoop() -> bool {
    command_exists("scoop")
        && CommandSpec::new("scoop")
            .args(["list", "azure-cli"])
            .run()
            .is_ok()
}

fn command_exists(program: &str) -> bool {
    crate::executable::command_exists(program)
}

#[cfg(test)]
mod tests {
    use super::{detect_az, AzClient, AzOutput, AzStep, AZ};
    use crate::options::RunOptions;
    use crate::step::{Detection, Operation, Step};
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    /// An `az` that replays recorded transcripts.
    struct FakeAz {
        upgrade: AzOutput,
        calls: Mutex<Vec<String>>,
    }

    impl FakeAz {
        fn new(upgrade: AzOutput) -> Self {
            Self {
                upgrade,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn ok() -> Self {
            Self::new(AzOutput::success(
                "You have the latest version of Azure CLI installed.\n",
            ))
        }

        fn failed() -> Self {
            Self::new(AzOutput::failure("ERROR: az upgrade failed\n"))
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }

        fn record(&self, call: &str) {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(call.to_owned());
            }
        }
    }

    impl AzClient for FakeAz {
        fn upgrade(&self) -> AzOutput {
            self.record("upgrade --yes");
            self.upgrade.clone()
        }
    }

    fn step_with(client: Arc<FakeAz>) -> AzStep {
        AzStep::with_client(client)
    }

    #[test]
    fn check_reports_up_to_date() {
        let step = step_with(Arc::new(FakeAz::ok()));
        assert!(step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"))
            .is_up_to_date());
    }

    #[test]
    fn run_upgrades_az() {
        let client = Arc::new(FakeAz::ok());
        let step = step_with(client.clone());
        assert!(step.run_with_options(&RunOptions::new()).is_ok());
        assert_eq!(client.calls(), ["upgrade --yes"]);
    }

    #[test]
    fn run_reports_a_failed_upgrade() {
        let client = Arc::new(FakeAz::failed());
        let step = step_with(client);
        let error = match step.run_with_options(&RunOptions::new()) {
            Err(error) => error,
            Ok(()) => panic!("expected a failed upgrade"),
        };
        assert_eq!(error.step(), AZ);
        assert_eq!(error.operation(), Operation::Run);
    }

    #[test]
    fn is_user_level() {
        assert!(AzStep::new().is_user_level());
    }

    /// A stub directory on `PATH` whose commands always exit 0 and log their
    /// invocation to `$FRESHEN_AZ_LOG`.
    fn stub_dir() -> (PathBuf, PathBuf) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("freshen-az-{}-{}", std::process::id(), id));
        fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        let log = dir.join("calls.log");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = "#!/bin/sh\n\
                 {\n\
                   printf '%s' \"$0\"\n\
                   for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
                   printf '\\n'\n\
                 } >> \"$FRESHEN_AZ_LOG\"\n\
                 exit 0\n";
            let stub = dir.join("az");
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\n>>\"%FRESHEN_AZ_LOG%\" echo %0 %*\r\nexit /b 0\r\n";
            let stub = dir.join("az.cmd");
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }

        (dir, log)
    }

    /// The `PATH` list separator for the current platform.
    const PATH_LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

    /// Runs `f` with `az` and the given package managers on `PATH`,
    /// restoring `PATH` afterwards.
    ///
    /// Holds `crate::process::PATH_GUARD` for the whole mutation so parallel
    /// tests cannot replace `PATH` (and wipe a stub such as `choco`) out from
    /// under another test.
    fn with_path_guard(pms: &[&str], f: impl FnOnce()) {
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = env::var("PATH").ok();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for pm in pms {
            let (dir, _log) = stub_dir();
            write_stub(pm, &dir);
            dirs.push(dir);
        }
        let (az_dir, _az_log) = stub_dir();
        write_stub("az", &az_dir);
        dirs.push(az_dir);

        let mut joined = String::new();
        for dir in &dirs {
            if !joined.is_empty() {
                joined.push(PATH_LIST_SEPARATOR);
            }
            joined.push_str(
                dir.to_str()
                    .unwrap_or_else(|| panic!("non-UTF-8 stub path: {}", dir.display())),
            );
        }
        if let Some(previous) = &previous {
            joined.push(PATH_LIST_SEPARATOR);
            joined.push_str(previous);
        }
        env::set_var("PATH", &joined);
        env::set_var("FRESHEN_AZ_LOG", "");

        let result = f;

        env::remove_var("FRESHEN_AZ_LOG");
        match previous {
            Some(previous) => env::set_var("PATH", &previous),
            None => env::remove_var("PATH"),
        }

        for dir in &dirs {
            let _ = fs::remove_dir_all(dir);
        }
        let _ = result;
    }

    fn write_stub(pm: &str, dir: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = "#!/bin/sh\nexit 0\n";
            let stub = dir.join(pm);
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|error| panic!("failed to make stub executable: {error}"));
        }

        #[cfg(windows)]
        {
            let script = "@echo off\r\nexit /b 0\r\n";
            let stub = dir.join(format!("{pm}.cmd"));
            fs::write(&stub, script)
                .unwrap_or_else(|error| panic!("failed to write stub {}: {error}", stub.display()));
        }
    }

    #[test]
    fn detect_reports_not_installed_when_az_is_absent() {
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = env::var("PATH").ok();
        env::set_var("PATH", "");
        let result = detect_az();
        match previous {
            Some(previous) => env::set_var("PATH", &previous),
            None => env::remove_var("PATH"),
        }
        assert_eq!(result, Detection::NotInstalled);
    }

    #[test]
    fn detect_reports_installed_when_az_is_not_system_managed() {
        with_path_guard(&[], || {
            assert_eq!(detect_az(), Detection::Installed);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_brew_owns_az() {
        // Put the stub az inside a fake Homebrew prefix, next to a stub
        // `bin/brew` so Homebrew reports itself as installed at that prefix.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = env::var("PATH").ok();
        let dir = env::temp_dir().join(format!("freshen-az-brew-{}-{}", std::process::id(), id));
        fs::create_dir_all(dir.join("bin")).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        write_stub("az", dir.join("bin").as_path());
        write_stub("brew", dir.join("bin").as_path());
        env::set_var(
            "FRESHEN_BREW_PREFIXES",
            dir.to_str()
                .unwrap_or_else(|| panic!("non-UTF-8 stub path: {}", dir.display())),
        );
        env::set_var(
            "PATH",
            dir.join("bin")
                .to_str()
                .unwrap_or_else(|| panic!("non-UTF-8 stub path: {}", dir.display())),
        );

        let result = detect_az();

        env::remove_var("FRESHEN_BREW_PREFIXES");
        match previous {
            Some(previous) => env::set_var("PATH", &previous),
            None => env::remove_var("PATH"),
        }
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result, Detection::NotInstalled);
    }

    #[test]
    fn detect_reports_not_installed_when_dpkg_owns_az() {
        with_path_guard(&["dpkg"], || {
            assert_eq!(detect_az(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_dnf_owns_az() {
        with_path_guard(&["dnf"], || {
            assert_eq!(detect_az(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_pacman_owns_az() {
        with_path_guard(&["pacman"], || {
            assert_eq!(detect_az(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_winget_owns_az() {
        with_path_guard(&["winget"], || {
            assert_eq!(detect_az(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_choco_owns_az() {
        with_path_guard(&["choco"], || {
            assert_eq!(detect_az(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_reports_not_installed_when_scoop_owns_az() {
        with_path_guard(&["scoop"], || {
            assert_eq!(detect_az(), Detection::NotInstalled);
        });
    }

    #[test]
    fn detect_still_reports_installed_when_an_unrelated_package_manager_is_present() {
        with_path_guard(&["apt"], || {
            assert_eq!(detect_az(), Detection::Installed);
        });
    }
}
