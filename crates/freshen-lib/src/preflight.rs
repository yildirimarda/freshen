//! Preflight checks that run before an update starts.
//!
//! Updating a machine needs a working network, enough room for downloads and
//! an environment where it is reasonable to start. These checks catch the
//! common cases before the first package manager changes anything:
//!
//! * the network is reachable, honouring the proxy and custom CA variables
//!   from the environment;
//! * enough free disk space is available;
//! * the machine is not running on a low battery;
//! * the connection is not metered or a phone hotspot.
//!
//! Every check is best effort: a fact the host cannot determine (no battery,
//! no NetworkManager, an unknown network cost) never blocks a run. Each check
//! can be disabled independently through [`PreflightConfig`], and the host
//! implementation is separated behind [`PreflightProbe`] so tests can inject
//! fixed facts instead of reading the real machine.

use std::fmt;
use std::path::PathBuf;
use std::process::Command;

/// Default minimum free disk space, in GiB.
pub const DEFAULT_MIN_FREE_DISK_GIB: u64 = 1;

/// Default minimum free disk space, in bytes (1 GiB).
pub const DEFAULT_MIN_FREE_DISK_BYTES: u64 = DEFAULT_MIN_FREE_DISK_GIB * 1024 * 1024 * 1024;

/// Default battery threshold, in percent.
pub const DEFAULT_MIN_BATTERY_PERCENT: u8 = 20;

/// Environment variable that disables every pre-run check when set to a
/// truthy value: the preflight checks and the other-package-manager check.
/// Used by the stub test harness so unrelated runs never touch the host
/// network, battery or process table.
pub const SKIP_ENV: &str = "FRESHEN_SKIP_PREFLIGHT";

/// One of the checks that can run before an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreflightCheck {
    /// The network can reach a known host.
    Network,
    /// Enough disk space is free.
    DiskSpace,
    /// The machine is not on a low battery.
    Battery,
    /// The connection is not metered or a hotspot.
    MeteredConnection,
}

impl PreflightCheck {
    /// Every check, in report order.
    pub const ALL: [Self; 4] = [
        Self::Network,
        Self::DiskSpace,
        Self::Battery,
        Self::MeteredConnection,
    ];

    /// Stable lowercase id used in messages and tests.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::DiskSpace => "disk space",
            Self::Battery => "battery",
            Self::MeteredConnection => "metered connection",
        }
    }
}

impl fmt::Display for PreflightCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which checks run and the thresholds they use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightConfig {
    network: bool,
    disk_space: bool,
    battery: bool,
    metered: bool,
    min_free_disk_bytes: u64,
    min_battery_percent: u8,
}

impl Default for PreflightConfig {
    fn default() -> Self {
        Self {
            network: true,
            disk_space: true,
            battery: true,
            metered: true,
            min_free_disk_bytes: DEFAULT_MIN_FREE_DISK_BYTES,
            min_battery_percent: DEFAULT_MIN_BATTERY_PERCENT,
        }
    }
}

impl PreflightConfig {
    /// Creates the default configuration: every check enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables one check.
    #[must_use]
    pub fn with_check(mut self, check: PreflightCheck, enabled: bool) -> Self {
        match check {
            PreflightCheck::Network => self.network = enabled,
            PreflightCheck::DiskSpace => self.disk_space = enabled,
            PreflightCheck::Battery => self.battery = enabled,
            PreflightCheck::MeteredConnection => self.metered = enabled,
        }
        self
    }

    /// Sets the minimum amount of free disk space required, in bytes.
    #[must_use]
    pub fn with_min_free_disk_bytes(mut self, bytes: u64) -> Self {
        self.min_free_disk_bytes = bytes;
        self
    }

    /// Sets the minimum battery percentage allowed while discharging.
    #[must_use]
    pub fn with_min_battery_percent(mut self, percent: u8) -> Self {
        self.min_battery_percent = percent;
        self
    }

    /// Returns `true` when `check` is enabled.
    #[must_use]
    pub fn is_enabled(&self, check: PreflightCheck) -> bool {
        match check {
            PreflightCheck::Network => self.network,
            PreflightCheck::DiskSpace => self.disk_space,
            PreflightCheck::Battery => self.battery,
            PreflightCheck::MeteredConnection => self.metered,
        }
    }

    /// Minimum free disk space required, in bytes.
    #[must_use]
    pub fn min_free_disk_bytes(&self) -> u64 {
        self.min_free_disk_bytes
    }

    /// Minimum battery percentage allowed while discharging.
    #[must_use]
    pub fn min_battery_percent(&self) -> u8 {
        self.min_battery_percent
    }
}

/// Battery charge and whether the machine is currently plugged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    percent: u8,
    charging: bool,
}

impl BatteryStatus {
    /// Creates a battery status.
    #[must_use]
    pub fn new(percent: u8, charging: bool) -> Self {
        Self { percent, charging }
    }

    /// Charge left, in percent.
    #[must_use]
    pub fn percent(&self) -> u8 {
        self.percent
    }

    /// Returns `true` while the battery is charging or fully charged.
    #[must_use]
    pub fn is_charging(&self) -> bool {
        self.charging
    }
}

/// Facts about the host that the preflight checks need.
///
/// Tests provide fixed facts through a fake probe; [`HostPreflightProbe`]
/// reads the real machine. Returning `None` means "cannot determine", which
/// never fails a check.
pub trait PreflightProbe {
    /// Whether a known host on the internet can be reached.
    fn network_reachable(&self) -> bool;

    /// Why the network check failed, or `Ok(())` when it passed.
    ///
    /// Probes that can explain the failure override this; the default keeps
    /// the generic message for scripted facts.
    fn network_status(&self) -> Result<(), String> {
        if self.network_reachable() {
            Ok(())
        } else {
            Err("network is unreachable".to_owned())
        }
    }

    /// Free space on the volume freshen writes to, in bytes.
    fn free_disk_bytes(&self) -> Option<u64>;

    /// Battery state, or `None` on a machine without a battery.
    fn battery(&self) -> Option<BatteryStatus>;

    /// Whether the active connection is metered or a hotspot, or `None` when
    /// the host cannot tell.
    fn metered_connection(&self) -> Option<bool>;
}

/// One failed preflight check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightIssue {
    check: PreflightCheck,
    message: String,
}

impl PreflightIssue {
    fn new(check: PreflightCheck, message: impl Into<String>) -> Self {
        Self {
            check,
            message: message.into(),
        }
    }

    /// The check that failed.
    #[must_use]
    pub fn check(&self) -> PreflightCheck {
        self.check
    }

    /// Human-readable explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PreflightIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.check, self.message)
    }
}

/// The collected result of every enabled preflight check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreflightReport {
    issues: Vec<PreflightIssue>,
}

impl PreflightReport {
    /// Runs every enabled check against `probe`.
    #[must_use]
    pub fn run(probe: &dyn PreflightProbe, config: &PreflightConfig) -> Self {
        let mut issues = Vec::new();

        if config.is_enabled(PreflightCheck::Network) {
            if let Err(message) = probe.network_status() {
                issues.push(PreflightIssue::new(PreflightCheck::Network, message));
            }
        }

        if config.is_enabled(PreflightCheck::DiskSpace) {
            if let Some(free) = probe.free_disk_bytes() {
                if free < config.min_free_disk_bytes() {
                    issues.push(PreflightIssue::new(
                        PreflightCheck::DiskSpace,
                        format!(
                            "only {} free, need at least {}",
                            format_gib(free),
                            format_gib(config.min_free_disk_bytes())
                        ),
                    ));
                }
            }
        }

        if config.is_enabled(PreflightCheck::Battery) {
            if let Some(battery) = probe.battery() {
                if !battery.is_charging() && battery.percent() < config.min_battery_percent() {
                    issues.push(PreflightIssue::new(
                        PreflightCheck::Battery,
                        format!(
                            "running on battery at {}%, below the {}% threshold",
                            battery.percent(),
                            config.min_battery_percent()
                        ),
                    ));
                }
            }
        }

        if config.is_enabled(PreflightCheck::MeteredConnection)
            && probe.metered_connection() == Some(true)
        {
            issues.push(PreflightIssue::new(
                PreflightCheck::MeteredConnection,
                "connection is metered or a hotspot",
            ));
        }

        Self { issues }
    }

    /// Returns `true` when every enabled check passed.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// The checks that failed, in report order.
    #[must_use]
    pub fn issues(&self) -> &[PreflightIssue] {
        &self.issues
    }
}

impl fmt::Display for PreflightReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_ok() {
            return writeln!(f, "Preflight checks passed.");
        }

        writeln!(f, "Preflight checks failed:")?;
        for issue in &self.issues {
            writeln!(f, "  - {issue}")?;
        }
        Ok(())
    }
}

fn format_gib(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gib = bytes as f64 / GIB;
    if gib >= 10.0 {
        format!("{gib:.0} GiB")
    } else {
        format!("{gib:.1} GiB")
    }
}

/// Reads preflight facts from the real machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostPreflightProbe;

impl PreflightProbe for HostPreflightProbe {
    fn network_reachable(&self) -> bool {
        crate::network::check().is_ok()
    }

    fn network_status(&self) -> Result<(), String> {
        crate::network::check().map_err(|error| error.to_string())
    }

    fn free_disk_bytes(&self) -> Option<u64> {
        let path = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        fs2::available_space(path).ok()
    }

    fn battery(&self) -> Option<BatteryStatus> {
        #[cfg(target_os = "linux")]
        {
            linux_battery()
        }
        #[cfg(target_os = "macos")]
        {
            macos_battery()
        }
        #[cfg(windows)]
        {
            windows_battery()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            None
        }
    }

    fn metered_connection(&self) -> Option<bool> {
        #[cfg(target_os = "linux")]
        {
            linux_metered()
        }
        #[cfg(target_os = "macos")]
        {
            macos_metered()
        }
        #[cfg(windows)]
        {
            windows_metered()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            None
        }
    }
}

#[cfg(test)]
fn reachable(endpoints: &[std::net::SocketAddr], timeout: std::time::Duration) -> bool {
    endpoints
        .iter()
        .any(|endpoint| std::net::TcpStream::connect_timeout(endpoint, timeout).is_ok())
}

#[cfg(target_os = "linux")]
fn linux_battery() -> Option<BatteryStatus> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut percent: Option<u8> = None;
    let mut all_charging = true;
    let mut found = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if !kind.trim().eq_ignore_ascii_case("Battery") {
            continue;
        }
        let Ok(capacity) = std::fs::read_to_string(path.join("capacity")) else {
            continue;
        };
        let Ok(capacity) = capacity.trim().parse::<u8>() else {
            continue;
        };
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        let charging = status.trim().eq_ignore_ascii_case("Charging")
            || status.trim().eq_ignore_ascii_case("Full");

        found = true;
        percent = Some(percent.map_or(capacity, |current| current.min(capacity)));
        all_charging &= charging;
    }

    found.then(|| BatteryStatus::new(percent.unwrap_or_default(), all_charging))
}

#[cfg(any(target_os = "macos", test))]
fn parse_pmset_battery(output: &str) -> Option<BatteryStatus> {
    for line in output.lines() {
        let Some((before, rest)) = line.split_once('%') else {
            continue;
        };
        let digits: String = before
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let Ok(percent) = digits.parse::<u8>() else {
            continue;
        };
        let state = rest
            .split(';')
            .nth(1)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let charging = (state.contains("charg") && !state.contains("discharg"))
            || state.contains("finishing")
            || state.contains("ac");
        return Some(BatteryStatus::new(percent, charging));
    }
    None
}

#[cfg(target_os = "macos")]
fn macos_battery() -> Option<BatteryStatus> {
    let output = Command::new("pmset").args(["-g", "batt"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_pmset_battery(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(windows, test))]
fn parse_windows_battery(output: &str) -> Option<BatteryStatus> {
    for line in output.lines() {
        let Some((percent, status)) = line.trim().split_once(';') else {
            continue;
        };
        let Ok(percent) = percent.trim().parse::<u8>() else {
            continue;
        };
        let Ok(status) = status.trim().parse::<u32>() else {
            continue;
        };
        let charging = matches!(status, 2 | 6 | 7 | 8 | 9);
        return Some(BatteryStatus::new(percent, charging));
    }
    None
}

#[cfg(windows)]
fn windows_battery() -> Option<BatteryStatus> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Battery | ForEach-Object { \"$($_.EstimatedChargeRemaining);$($_.BatteryStatus)\" }",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_windows_battery(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(target_os = "linux", test))]
fn parse_nmcli_metered(output: &str) -> Option<bool> {
    let mut seen = false;
    let mut metered = false;

    for line in output.lines() {
        let Some((_, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_ascii_lowercase();
        if value.starts_with("yes") {
            seen = true;
            metered = true;
        } else if value.starts_with("no") {
            seen = true;
        }
    }

    seen.then_some(metered)
}

#[cfg(target_os = "linux")]
fn linux_metered() -> Option<bool> {
    let output = Command::new("nmcli")
        .args(["-t", "-f", "GENERAL.METERED", "device", "show"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_nmcli_metered(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(target_os = "macos", test))]
fn is_hotspot_ssid(ssid: &str) -> bool {
    let ssid = ssid.to_ascii_lowercase();
    ["iphone", "android", "hotspot", "personal hotspot", "tether"]
        .iter()
        .any(|keyword| ssid.contains(keyword))
}

#[cfg(target_os = "macos")]
fn macos_metered() -> Option<bool> {
    let route = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    if !route.status.success() {
        return None;
    }
    let route = String::from_utf8_lossy(&route.stdout);
    let interface = route
        .lines()
        .find_map(|line| line.trim().strip_prefix("interface:").map(str::trim))?;

    if interface.starts_with("pdp_ip") {
        return Some(true);
    }

    let wifi = Command::new("networksetup")
        .args(["-getairportnetwork", interface])
        .output()
        .ok()?;
    if !wifi.status.success() {
        return None;
    }
    let wifi = String::from_utf8_lossy(&wifi.stdout);
    let ssid = wifi.split_once("Current Wi-Fi Network:")?.1.trim();
    Some(is_hotspot_ssid(ssid))
}

#[cfg(any(windows, test))]
fn parse_windows_network_cost(output: &str) -> Option<bool> {
    let mut seen = false;
    let mut metered = false;

    for line in output.lines() {
        match line.trim().to_ascii_lowercase().as_str() {
            "unrestricted" => seen = true,
            "fixed" | "variable" => {
                seen = true;
                metered = true;
            }
            _ => {}
        }
    }

    seen.then_some(metered)
}

#[cfg(windows)]
fn windows_metered() -> Option<bool> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-NetConnectionProfile | ForEach-Object { $_.NetworkCostType }",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_windows_network_cost(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::{
        format_gib, is_hotspot_ssid, parse_nmcli_metered, parse_pmset_battery,
        parse_windows_battery, parse_windows_network_cost, reachable, BatteryStatus,
        PreflightCheck, PreflightConfig, PreflightProbe, PreflightReport,
    };
    use std::net::{SocketAddr, TcpListener};
    use std::time::Duration;

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

    fn issue_checks(report: &PreflightReport) -> Vec<PreflightCheck> {
        report.issues().iter().map(|issue| issue.check()).collect()
    }

    #[test]
    fn defaults_enable_every_check_with_sensible_thresholds() {
        let config = PreflightConfig::new();

        for check in PreflightCheck::ALL {
            assert!(config.is_enabled(check), "{check}");
        }
        assert_eq!(config.min_free_disk_bytes(), 1024 * 1024 * 1024);
        assert_eq!(config.min_battery_percent(), 20);
    }

    #[test]
    fn all_clear_when_the_host_is_healthy() {
        let report = PreflightReport::run(&FakeProbe::default(), &PreflightConfig::new());

        assert!(report.is_ok());
        assert!(report.issues().is_empty());
        assert_eq!(report.to_string(), "Preflight checks passed.\n");
    }

    #[test]
    fn an_unreachable_network_is_reported() {
        let probe = FakeProbe {
            network: false,
            ..FakeProbe::default()
        };

        let report = PreflightReport::run(&probe, &PreflightConfig::new());

        assert!(!report.is_ok());
        assert_eq!(issue_checks(&report), [PreflightCheck::Network]);
        assert!(report.to_string().contains("network is unreachable"));
    }

    #[test]
    fn a_specific_network_failure_is_reported() {
        let probe = FakeProbe {
            network: false,
            network_error: Some("invalid proxy URL: unsupported proxy scheme 'socks5'".to_owned()),
            ..FakeProbe::default()
        };

        let report = PreflightReport::run(&probe, &PreflightConfig::new());

        assert!(!report.is_ok());
        assert_eq!(issue_checks(&report), [PreflightCheck::Network]);
        assert!(report
            .to_string()
            .contains("invalid proxy URL: unsupported proxy scheme 'socks5'"));
        assert!(!report.to_string().contains("network is unreachable"));
    }

    #[test]
    fn free_disk_space_below_the_threshold_is_reported() {
        let probe = FakeProbe {
            disk: Some(512 * 1024 * 1024),
            ..FakeProbe::default()
        };

        let report = PreflightReport::run(&probe, &PreflightConfig::new());

        assert_eq!(issue_checks(&report), [PreflightCheck::DiskSpace]);
        assert!(report.to_string().contains("only 0.5 GiB free"));
    }

    #[test]
    fn exactly_enough_disk_space_passes() {
        let config = PreflightConfig::new();
        let probe = FakeProbe {
            disk: Some(config.min_free_disk_bytes()),
            ..FakeProbe::default()
        };

        let report = PreflightReport::run(&probe, &config);

        assert!(report.is_ok());
    }

    #[test]
    fn a_discharging_battery_below_the_threshold_is_reported() {
        let probe = FakeProbe {
            battery: Some(BatteryStatus::new(10, false)),
            ..FakeProbe::default()
        };

        let report = PreflightReport::run(&probe, &PreflightConfig::new());

        assert_eq!(issue_checks(&report), [PreflightCheck::Battery]);
        assert!(report.to_string().contains("running on battery at 10%"));
    }

    #[test]
    fn a_charging_battery_or_one_without_a_battery_never_blocks() {
        let charging = FakeProbe {
            battery: Some(BatteryStatus::new(5, true)),
            ..FakeProbe::default()
        };
        let no_battery = FakeProbe::default();

        assert!(PreflightReport::run(&charging, &PreflightConfig::new()).is_ok());
        assert!(PreflightReport::run(&no_battery, &PreflightConfig::new()).is_ok());
    }

    #[test]
    fn a_battery_at_the_threshold_passes() {
        let probe = FakeProbe {
            battery: Some(BatteryStatus::new(20, false)),
            ..FakeProbe::default()
        };

        assert!(PreflightReport::run(&probe, &PreflightConfig::new()).is_ok());
    }

    #[test]
    fn a_metered_connection_is_reported() {
        let probe = FakeProbe {
            metered: Some(true),
            ..FakeProbe::default()
        };

        let report = PreflightReport::run(&probe, &PreflightConfig::new());

        assert_eq!(issue_checks(&report), [PreflightCheck::MeteredConnection]);
    }

    #[test]
    fn an_unmetered_or_unknown_connection_passes() {
        let unmetered = FakeProbe {
            metered: Some(false),
            ..FakeProbe::default()
        };
        let unknown = FakeProbe::default();

        assert!(PreflightReport::run(&unmetered, &PreflightConfig::new()).is_ok());
        assert!(PreflightReport::run(&unknown, &PreflightConfig::new()).is_ok());
    }

    #[test]
    fn each_check_can_be_disabled_independently() {
        let probe = FakeProbe {
            network: false,
            network_error: None,
            disk: Some(0),
            battery: Some(BatteryStatus::new(1, false)),
            metered: Some(true),
        };

        for check in PreflightCheck::ALL {
            let config = PreflightConfig::new().with_check(check, false);
            let report = PreflightReport::run(&probe, &config);

            assert_eq!(report.issues().len(), 3, "{check}");
            assert!(!issue_checks(&report).contains(&check), "{check}");
        }
    }

    #[test]
    fn custom_thresholds_are_applied() {
        let probe = FakeProbe {
            disk: Some(3 * 1024 * 1024 * 1024),
            battery: Some(BatteryStatus::new(30, false)),
            ..FakeProbe::default()
        };
        let config = PreflightConfig::new()
            .with_min_free_disk_bytes(4 * 1024 * 1024 * 1024)
            .with_min_battery_percent(40);

        let report = PreflightReport::run(&probe, &config);

        assert_eq!(
            issue_checks(&report),
            [PreflightCheck::DiskSpace, PreflightCheck::Battery]
        );
    }

    #[test]
    fn every_failure_is_reported_together_in_order() {
        let probe = FakeProbe {
            network: false,
            network_error: None,
            disk: Some(0),
            battery: Some(BatteryStatus::new(5, false)),
            metered: Some(true),
        };

        let report = PreflightReport::run(&probe, &PreflightConfig::new());

        assert_eq!(issue_checks(&report), PreflightCheck::ALL);
        assert_eq!(report.issues().len(), 4);
    }

    #[test]
    fn gib_formatting_is_readable() {
        assert_eq!(format_gib(0), "0.0 GiB");
        assert_eq!(format_gib(512 * 1024 * 1024), "0.5 GiB");
        assert_eq!(format_gib(12 * 1024 * 1024 * 1024), "12 GiB");
    }

    /// Ask the kernel for an ephemeral loopback port and release it again.
    ///
    /// Another process can claim the port as soon as the listener is dropped,
    /// so callers must be prepared for the returned address to be reachable.
    fn released_loopback_addr() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("failed to bind test listener: {error}"));
        listener
            .local_addr()
            .unwrap_or_else(|error| panic!("failed to read test listener address: {error}"))
    }

    #[test]
    fn tcp_reachability_detects_open_and_closed_ports() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("failed to bind test listener: {error}"));
        let open = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("failed to read test listener address: {error}"));
        assert!(reachable(&[open], Duration::from_millis(500)));

        drop(listener);

        // A busy CI runner may hand the just-released port to another process
        // before the probe runs. Retry with fresh ephemeral ports until one is
        // still closed instead of assuming the first candidate stayed free.
        let closed = (0..32).find_map(|_| {
            let candidate = released_loopback_addr();
            (!reachable(&[candidate], Duration::from_millis(500))).then_some(candidate)
        });
        assert!(
            closed.is_some(),
            "no closed loopback port found after 32 attempts"
        );
    }

    #[test]
    fn pmset_output_is_parsed() {
        let discharging = "Now drawing from 'Battery Power'\n \
            -InternalBattery-0 (id=12345)\t42%; discharging; 3:12 remaining present: true\n";
        let charging = "Now drawing from 'AC Power'\n \
            -InternalBattery-0 (id=12345)\t80%; charged; 0:00 remaining present: true\n";

        assert_eq!(
            parse_pmset_battery(discharging),
            Some(BatteryStatus::new(42, false))
        );
        assert_eq!(
            parse_pmset_battery(charging),
            Some(BatteryStatus::new(80, true))
        );
        assert_eq!(parse_pmset_battery("no battery here"), None);
    }

    #[test]
    fn windows_battery_output_is_parsed() {
        assert_eq!(
            parse_windows_battery("55;1\n"),
            Some(BatteryStatus::new(55, false))
        );
        assert_eq!(
            parse_windows_battery("100;2\n"),
            Some(BatteryStatus::new(100, true))
        );
        assert_eq!(parse_windows_battery("garbage"), None);
    }

    #[test]
    fn nmcli_metered_output_is_parsed() {
        assert_eq!(parse_nmcli_metered("GENERAL.METERED:yes\n"), Some(true));
        assert_eq!(
            parse_nmcli_metered("GENERAL.METERED:no (guessed)\n"),
            Some(false)
        );
        assert_eq!(parse_nmcli_metered("GENERAL.METERED:unknown\n"), None);
        assert_eq!(parse_nmcli_metered(""), None);
    }

    #[test]
    fn windows_network_cost_is_parsed() {
        assert_eq!(parse_windows_network_cost("Unrestricted\n"), Some(false));
        assert_eq!(parse_windows_network_cost("Fixed\n"), Some(true));
        assert_eq!(parse_windows_network_cost("Variable\n"), Some(true));
        assert_eq!(parse_windows_network_cost("Unknown\n"), None);
        assert_eq!(parse_windows_network_cost(""), None);
    }

    #[test]
    fn hotspot_ssids_are_recognised() {
        assert!(is_hotspot_ssid("Arda's iPhone"));
        assert!(is_hotspot_ssid("AndroidAP"));
        assert!(is_hotspot_ssid("My Personal Hotspot"));
        assert!(!is_hotspot_ssid("Home Wi-Fi"));
    }

    #[test]
    fn reachability_helper_handles_several_endpoints() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("failed to bind test listener: {error}"));
        let open = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("failed to read test listener address: {error}"));
        let closed = SocketAddr::from(([127, 0, 0, 1], 1));

        assert!(reachable(&[closed, open], Duration::from_millis(500)));
    }
}
