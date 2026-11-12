//! `freshen --doctor`: verifies the environment and prints actionable fixes.
//!
//! The doctor checks that the tools freshen uses are installed, that freshen
//! has the permissions it needs, and that the configuration file is valid.
//! Every problem comes with a concrete fix the user can apply.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::config::ConfigError;
use crate::options::RunOptions;
use crate::platform::Platform;
use crate::preflight::{PreflightConfig, PreflightProbe, PreflightReport};
use crate::registry::Registry;
use crate::step::Detection;
use crate::system::Privilege;

/// One problem the doctor found and the action that fixes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorFinding {
    /// Area of the environment the finding is about (config, a step, etc.).
    pub area: String,
    /// What is wrong.
    pub problem: String,
    /// What the user can do to fix it.
    pub fix: String,
    /// Whether the finding blocks a freshen run.
    pub blocking: bool,
}

impl DoctorFinding {
    /// Creates a non-blocking informational finding.
    #[must_use]
    pub fn info(
        area: impl Into<String>,
        problem: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            area: area.into(),
            problem: problem.into(),
            fix: fix.into(),
            blocking: false,
        }
    }

    /// Creates a blocking finding.
    #[must_use]
    pub fn error(
        area: impl Into<String>,
        problem: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            area: area.into(),
            problem: problem.into(),
            fix: fix.into(),
            blocking: true,
        }
    }

    /// Returns `true` when the finding blocks a freshen run.
    #[must_use]
    pub fn blocking(&self) -> bool {
        self.blocking
    }
}

/// The collected result of every doctor check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoctorReport {
    findings: Vec<DoctorFinding>,
}

impl DoctorReport {
    /// Adds a finding.
    fn push(&mut self, finding: DoctorFinding) {
        self.findings.push(finding);
    }

    /// Returns `true` when no blocking finding was reported.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        !self.findings.iter().any(|finding| finding.blocking())
    }
    /// Returns `true` when no finding at all was reported.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// Every finding, in report order.
    #[must_use]
    pub fn findings(&self) -> &[DoctorFinding] {
        &self.findings
    }

    /// Exit code for doctor mode: `0` when the environment is healthy, `1`
    /// when at least one blocking problem was found.
    #[must_use]
    pub fn exit_code(&self) -> std::process::ExitCode {
        if self.is_healthy() {
            crate::exit::code(crate::exit::OK)
        } else {
            crate::exit::code(crate::exit::STEP_FAILURE)
        }
    }
}

impl fmt::Display for DoctorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.findings.is_empty() {
            return writeln!(f, "freshen doctor: everything looks good.");
        }

        let blocking = self
            .findings
            .iter()
            .filter(|finding| finding.blocking())
            .count();
        let informational = self.findings.len() - blocking;

        if blocking > 0 {
            writeln!(
                f,
                "freshen doctor: {blocking} problem(s) found, {informational} informational."
            )?;
        } else {
            writeln!(
                f,
                "freshen doctor: no problems found ({informational} informational)."
            )?;
        }

        for finding in &self.findings {
            let marker = if finding.blocking {
                "[error]"
            } else {
                "[info]"
            };
            writeln!(f, "  {marker} {}: {}", finding.area, finding.problem)?;
            if !finding.fix.is_empty() {
                writeln!(f, "        fix: {}", finding.fix)?;
            }
        }
        Ok(())
    }
}

/// Runs every doctor check and returns the collected report.
///
/// `config_error` is the error from loading the configuration file, when any.
/// `preflight` probes the host for network, disk, battery and metered-connection
/// facts; tests inject a fake probe instead of reading the real machine.
pub fn doctor(
    registry: &Registry,
    options: &RunOptions,
    platform: Platform,
    config_path: &Path,
    config_error: Option<&ConfigError>,
    privilege: Privilege,
    preflight: &dyn PreflightProbe,
) -> DoctorReport {
    let mut report = DoctorReport::default();

    config_check(&mut report, config_path, config_error);
    permissions_check(&mut report, config_path);
    privilege_check(&mut report, privilege);
    tools_check(&mut report, registry, platform);

    preflight_check(&mut report, options.preflight(), preflight);

    report
}

/// Reports whether the configuration file is valid and readable.
fn config_check(report: &mut DoctorReport, config_path: &Path, config_error: Option<&ConfigError>) {
    match config_error {
        Some(ConfigError::Invalid {
            path,
            line,
            column,
            message,
        }) => {
            report.push(DoctorFinding::error(
                "config",
                format!("configuration file is invalid: {message}"),
                format!(
                    "Fix the TOML syntax in {} at line {line}, column {column}: {message}",
                    path.display()
                ),
            ));
        }
        Some(ConfigError::Missing { path }) => {
            report.push(DoctorFinding::error(
                "config",
                format!("configuration file does not exist: {}", path.display()),
                "Create the file or drop --config if you did not mean to point at it."
                    .to_string(),
            ));
        }
        Some(ConfigError::Read { path, source }) => {
            report.push(DoctorFinding::error(
                "config",
                format!("could not read configuration file: {source}"),
                format!("Check that {} exists and is readable.", path.display()),
            ));
        }
        None => {
            if config_path.exists() {
                report.push(DoctorFinding::info(
                    "config",
                    "configuration file is valid",
                    format!(
                        "freshen is using {config_path}.",
                        config_path = config_path.display()
                    ),
                ));
            } else {
                report.push(DoctorFinding::info(
                    "config",
                    "no configuration file found; using built-in defaults",
                    format!("Create {} to customize freshen.", config_path.display()),
                ));
            }
        }
    }
}

/// Reports whether freshen can write to its configuration directory.
fn permissions_check(report: &mut DoctorReport, config_path: &Path) {
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));

    // The configuration directory is created by freshen when it writes
    // configuration, so a missing directory is not a permissions problem.
    if !config_dir.exists() {
        return;
    }

    let test_file = config_dir.join(".freshen-doctor-write-test");

    let writable = (|| -> io::Result<()> {
        let mut file = fs::File::create(&test_file)?;
        use std::io::Write as _;
        file.write_all(b"freshen-doctor")?;
        drop(file);
        fs::remove_file(&test_file)?;
        Ok(())
    })();

    if let Err(error) = writable {
        report.push(DoctorFinding::error(
            "permissions",
            format!("configuration directory is not writable: {error}"),
            format!(
                "Make {} writable (or point --config at a writable location).",
                config_dir.display()
            ),
        ));
    }
}

/// Reports the privilege level and what it means for the steps freshen can run.
fn privilege_check(report: &mut DoctorReport, privilege: Privilege) {
    let fix = if privilege.is_elevated() {
        String::new()
    } else {
        "Run freshen elevated (sudo / administrator) so system-level steps can run.".to_owned()
    };
    report.push(DoctorFinding::info(
        "privilege",
        format!("running with {} privileges", privilege),
        fix,
    ));
}

/// Reports whether every step's tool is detected on this machine.
fn tools_check(report: &mut DoctorReport, registry: &Registry, platform: Platform) {
    for step in registry.steps() {
        if !step.platforms().contains(&platform) {
            continue;
        }
        match step.detect() {
            Ok(Detection::Installed) => {}
            Ok(Detection::NotInstalled) => {
                report.push(DoctorFinding::info(
                    step.id(),
                    format!("{} is not installed", step.display_name()),
                    format!(
                        "Install {} so the `{}` step can run, or skip it with `--skip {}`.",
                        step.display_name(),
                        step.id(),
                        step.id()
                    ),
                ));
            }
            Ok(Detection::Unavailable(reason)) => {
                report.push(DoctorFinding::info(
                    step.id(),
                    format!(
                        "{} is installed but unavailable ({reason})",
                        step.display_name()
                    ),
                    format!(
                        "Resolve the `{reason}` condition so the `{}` step can run.",
                        step.id()
                    ),
                ));
            }
            Err(error) => {
                report.push(DoctorFinding::info(
                    step.id(),
                    format!("could not detect {}: {error}", step.display_name()),
                    format!("Fix the detection error for the `{}` step.", step.id()),
                ));
            }
        }
    }
}

/// Reports the preflight check results (network, disk, battery, metered).
fn preflight_check(
    report: &mut DoctorReport,
    preflight_config: &PreflightConfig,
    preflight: &dyn PreflightProbe,
) {
    let preflight_report = PreflightReport::run(preflight, preflight_config);
    for issue in preflight_report.issues() {
        report.push(DoctorFinding::error(
            "preflight",
            format!("{}: {}", issue.check(), issue.message()),
            preflight_fix(issue.check()),
        ));
    }
}

/// An actionable fix for a failed preflight check.
fn preflight_fix(check: crate::preflight::PreflightCheck) -> &'static str {
    match check {
        crate::preflight::PreflightCheck::Network => {
            "Check your internet connection, proxy settings (HTTP_PROXY/HTTPS_PROXY) \
             and CA bundle (SSL_CERT_FILE)."
        }
        crate::preflight::PreflightCheck::DiskSpace => {
            "Free up disk space or lower min_free_disk_gib in the configuration file."
        }
        crate::preflight::PreflightCheck::Battery => {
            "Plug the machine in or lower battery_threshold in the configuration file."
        }
        crate::preflight::PreflightCheck::MeteredConnection => {
            "Use a non-metered connection or disable the metered-connection check."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{doctor, DoctorFinding, DoctorReport};
    use crate::config::ConfigError;
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::preflight::{BatteryStatus, PreflightProbe};
    use crate::registry::Registry;
    use crate::step::{CheckReport, Detection, Step, StepError, StepId};
    use crate::system::Privilege;
    use std::path::PathBuf;

    /// A fake probe that returns fixed preflight facts.
    struct FakeProbe {
        network: bool,
        network_error: Option<String>,
        disk: Option<u64>,
        battery: Option<BatteryStatus>,
        metered: Option<bool>,
    }

    impl Default for FakeProbe {
        fn default() -> Self {
            Self {
                network: true,
                network_error: None,
                disk: None,
                battery: None,
                metered: None,
            }
        }
    }

    impl PreflightProbe for FakeProbe {
        fn network_reachable(&self) -> bool {
            self.network
        }

        fn network_status(&self) -> Result<(), String> {
            if let Some(error) = &self.network_error {
                return Err(error.clone());
            }
            if self.network {
                Ok(())
            } else {
                Err("network is unreachable".to_owned())
            }
        }

        fn free_disk_bytes(&self) -> Option<u64> {
            self.disk
        }

        fn battery(&self) -> Option<BatteryStatus> {
            self.battery
        }

        fn metered_connection(&self) -> Option<bool> {
            self.metered
        }
    }

    struct TestStep {
        id: StepId,
        name: &'static str,
        platforms: &'static [Platform],
        detection: Result<Detection, StepError>,
    }

    impl TestStep {
        fn new(id: StepId, platforms: &'static [Platform]) -> Self {
            Self {
                id,
                name: id,
                platforms,
                detection: Ok(Detection::Installed),
            }
        }

        #[allow(dead_code)]
        fn with_platforms(mut self, platforms: &'static [Platform]) -> Self {
            self.platforms = platforms;
            self
        }

        fn detection(mut self, detection: Result<Detection, StepError>) -> Self {
            self.detection = detection;
            self
        }
    }

    impl Step for TestStep {
        fn id(&self) -> StepId {
            self.id
        }

        fn display_name(&self) -> &str {
            self.name
        }

        fn detect(&self) -> Result<Detection, StepError> {
            self.detection.clone()
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            Ok(CheckReport::up_to_date())
        }

        fn run(&self) -> Result<(), StepError> {
            Ok(())
        }

        fn platforms(&self) -> &'static [Platform] {
            self.platforms
        }
    }

    fn registry(steps: impl IntoIterator<Item = TestStep>) -> Registry {
        steps
            .into_iter()
            .fold(Registry::new(), |registry, step| registry.with(step))
    }

    fn config_path() -> PathBuf {
        PathBuf::from("/tmp/freshen-doctor-test/config.toml")
    }

    fn run_doctor(
        steps: impl IntoIterator<Item = TestStep>,
        config_error: Option<ConfigError>,
        privilege: Privilege,
        preflight: FakeProbe,
    ) -> DoctorReport {
        let registry = registry(steps);
        let config_path = config_path();
        let options = RunOptions::new();
        doctor(
            &registry,
            &options,
            Platform::Linux,
            &config_path,
            config_error.as_ref(),
            privilege,
            &preflight,
        )
    }

    fn findings_by_area<'a>(report: &'a DoctorReport, area: &str) -> Vec<&'a DoctorFinding> {
        report
            .findings()
            .iter()
            .filter(|finding| finding.area == area)
            .collect()
    }

    #[test]
    fn healthy_environment_reports_nothing_blocking() {
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(report.is_healthy(), "report: {report}");
        assert_eq!(report.exit_code(), crate::exit::code(crate::exit::OK));
    }

    #[test]
    fn missing_tool_is_an_informational_finding() {
        let report = run_doctor(
            [
                TestStep::new("brew", &Platform::ALL),
                TestStep::new("uv", &Platform::ALL).detection(Ok(Detection::NotInstalled)),
            ],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(report.is_healthy());
        let missing = findings_by_area(&report, "uv");
        assert_eq!(missing.len(), 1);
        assert!(!missing[0].blocking);
        assert!(missing[0].problem.contains("is not installed"));
        assert!(!missing[0].fix.is_empty());
    }

    #[test]
    fn unsupported_steps_are_not_checked() {
        let report = run_doctor(
            [TestStep::new("mas", &[Platform::Macos]).detection(Ok(Detection::NotInstalled))],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(report.is_healthy());
        assert!(findings_by_area(&report, "mas").is_empty());
    }

    #[test]
    fn detection_error_is_an_informational_finding() {
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)
                .detection(Err(StepError::detect("brew", "command not found")))],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(report.is_healthy());
        assert_eq!(report.exit_code(), crate::exit::code(crate::exit::OK));
        let infos = findings_by_area(&report, "brew");
        assert_eq!(infos.len(), 1);
        assert!(!infos[0].blocking);
    }

    #[test]
    fn invalid_config_is_a_blocking_finding_that_points_at_the_line() {
        let error = ConfigError::Invalid {
            path: PathBuf::from("/tmp/freshen-doctor-test/config.toml"),
            line: 3,
            column: 10,
            message: "unknown field `foo`".to_owned(),
        };
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)],
            Some(error),
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(!report.is_healthy());
        let errors = findings_by_area(&report, "config");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].blocking);
        assert!(errors[0].problem.contains("unknown field `foo`"));
        assert!(errors[0].fix.contains("line 3, column 10"));
        assert_eq!(
            report.exit_code(),
            crate::exit::code(crate::exit::STEP_FAILURE)
        );
    }

    #[test]
    fn missing_explicit_config_is_a_blocking_finding() {
        let error = ConfigError::Missing {
            path: PathBuf::from("/tmp/freshen-doctor-test/config.toml"),
        };
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)],
            Some(error),
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(!report.is_healthy());
        let errors = findings_by_area(&report, "config");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].blocking);
        assert!(errors[0].problem.contains("does not exist"));
    }

    #[test]
    fn no_default_config_file_is_informational() {
        // `config_path()` points at a file that does not exist, and no error is
        // passed, so the doctor reports the optional default as informational.
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(report.is_healthy());
        let infos = findings_by_area(&report, "config");
        assert_eq!(infos.len(), 1);
        assert!(!infos[0].blocking);
        assert!(infos[0].problem.contains("no configuration file found"));
    }

    #[test]
    fn preflight_failures_are_reported() {
        let probe = FakeProbe {
            network: false,
            network_error: Some("invalid proxy URL: unsupported proxy scheme 'socks5'".to_owned()),
            disk: Some(512 * 1024 * 1024),
            battery: Some(BatteryStatus::new(10, false)),
            metered: Some(true),
        };
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)],
            None,
            Privilege::StandardUser,
            probe,
        );

        assert!(!report.is_healthy());
        let preflight = findings_by_area(&report, "preflight");
        assert_eq!(preflight.len(), 4);
        assert!(preflight.iter().all(|f| f.blocking));
        assert!(report
            .to_string()
            .contains("invalid proxy URL: unsupported proxy scheme 'socks5'"));
        assert!(report.to_string().contains("only 0.5 GiB free"));
        assert!(report.to_string().contains("running on battery at 10%"));
        assert!(report
            .to_string()
            .contains("connection is metered or a hotspot"));
    }

    #[test]
    fn privilege_is_reported() {
        let report = run_doctor(
            [TestStep::new("brew", &Platform::ALL)],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        let privileges = findings_by_area(&report, "privilege");
        assert_eq!(privileges.len(), 1);
        assert!(!privileges[0].blocking);
        assert!(privileges[0].problem.contains("standard"));
    }

    #[test]
    fn exit_code_is_one_when_a_blocking_finding_exists() {
        let error = ConfigError::Invalid {
            path: PathBuf::from("/tmp/x.toml"),
            line: 1,
            column: 1,
            message: "bad".to_owned(),
        };
        let report = run_doctor(
            [],
            Some(error),
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(!report.is_healthy());
        assert_eq!(
            report.exit_code(),
            crate::exit::code(crate::exit::STEP_FAILURE)
        );
    }

    #[test]
    fn exit_code_is_zero_when_only_informational_findings_exist() {
        let report = run_doctor(
            [TestStep::new("uv", &Platform::ALL).detection(Ok(Detection::NotInstalled))],
            None,
            Privilege::StandardUser,
            FakeProbe::default(),
        );

        assert!(report.is_healthy());
        assert_eq!(report.exit_code(), crate::exit::code(crate::exit::OK));
    }
}
