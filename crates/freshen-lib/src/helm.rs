//! The Helm step: `helm repo update`, skipped when helm is absent.
//!
//! The step runs `helm repo update` whenever `helm` is available. It is
//! skipped entirely when helm is absent, so it is safe to register on every
//! machine; on macOS it is never detected.

#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::executable::command_exists;
use crate::options::RunOptions;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::output::human;
use crate::platform::Platform;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::step::OutdatedItem;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};
use serde_json::Value;

/// The Helm package manager step.
pub const HELM: StepId = "helm";

/// Helm runs on Linux and Windows; on macOS it is never detected.
const HELM_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// The Helm releases step: a report-only list of Helm releases whose chart has
/// a newer version available in the configured repositories.
pub const HELM_RELEASES: StepId = "helm-releases";

/// Helm releases runs on Linux and Windows; on macOS it is never detected.
const HELM_RELEASES_PLATFORMS: [Platform; 2] = [Platform::Linux, Platform::Windows];

/// Registers the helm steps.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(HelmStep).with(HelmReleasesStep)
}

struct HelmStep;

impl HelmStep {
    /// Runs `helm repo update`.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("helm repo update");
        if let Err(error) = CommandSpec::new("helm")
            .args(["repo", "update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(HELM, &error));
        }
    }
}

impl Step for HelmStep {
    fn id(&self) -> StepId {
        HELM
    }

    fn display_name(&self) -> &str {
        "helm"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        return Ok(detect_helm());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    #[allow(unused_variables)]
    fn run_with_options(&self, _options: &RunOptions) -> Result<(), StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let mut failures = Vec::new();
            self.upgrade(&mut failures);
            finish(failures)
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            // helm is not registered on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &HELM_PLATFORMS
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Lists installed Helm releases and reports those with newer chart versions.
struct HelmReleasesStep;

impl Step for HelmReleasesStep {
    fn id(&self) -> StepId {
        HELM_RELEASES
    }

    fn display_name(&self) -> &str {
        "helm releases"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        return Ok(detect_helm());
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(Detection::NotInstalled);
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        return self.list_releases();
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        return Ok(CheckReport::up_to_date());
    }

    fn run(&self) -> Result<(), StepError> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.do_nothing();
            Ok(())
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            // Helm releases is not registered on macOS; nothing to run.
            Ok(())
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &HELM_RELEASES_PLATFORMS
    }

    fn is_user_level(&self) -> bool {
        true
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl HelmReleasesStep {
    /// Lists releases with `helm list` and reports those whose chart has a
    /// newer version available in the configured repositories.
    fn list_releases(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("helm")
            .args(["list", "-o", "json"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(HELM_RELEASES, &error))?;
        let releases = parse_helm_list(output.stdout());
        let mut items = Vec::new();
        for release in releases {
            let Some(latest) = self.latest_chart_version(&release.chart_name)? else {
                continue;
            };
            if version_newer(&latest, &release.chart_version) {
                items.push(
                    OutdatedItem::new(&release.name)
                        .with_current(&release.chart_version)
                        .with_latest(&latest)
                        .with_reason("newer chart version available"),
                );
            }
        }
        Ok(CheckReport::from_items(items))
    }

    /// The newest chart version available for `chart_name`, if any.
    fn latest_chart_version(&self, chart_name: &str) -> Result<Option<String>, StepError> {
        let output = CommandSpec::new("helm")
            .args(["search", "repo", chart_name, "-o", "json"])
            .neutral_locale()
            .run()
            .map_err(|error| StepError::check_from_process(HELM_RELEASES, &error))?;
        let Ok(Value::Array(items)) = serde_json::from_str(output.stdout()) else {
            return Ok(None);
        };
        let mut latest: Option<String> = None;
        for item in items {
            let Value::Object(obj) = item else { continue };
            let Some(chart) = obj.get("chart").and_then(Value::as_str) else {
                continue;
            };
            let basename = chart.rsplit('/').next().unwrap_or(chart);
            let result_name = split_chart(basename)
                .map(|(name, _)| name)
                .unwrap_or(basename);
            if result_name != chart_name {
                continue;
            }
            let Some(version) = obj.get("version").and_then(Value::as_str) else {
                continue;
            };
            if version.is_empty() {
                continue;
            }
            let is_newer = match &latest {
                None => true,
                Some(current) => version_newer(version, current),
            };
            if is_newer {
                latest = Some(version.to_owned());
            }
        }
        Ok(latest)
    }

    /// Report-only: never upgrades any release.
    fn do_nothing(&self) {
        human!("helm releases: report only, nothing to upgrade");
    }
}

/// An installed Helm release with the chart it tracks.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[derive(Debug, Clone)]
struct HelmRelease {
    name: String,
    chart_name: String,
    chart_version: String,
}

/// Parses `helm list -o json` output into the releases it lists.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn parse_helm_list(output: &str) -> Vec<HelmRelease> {
    let Ok(Value::Array(items)) = serde_json::from_str(output) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let Value::Object(obj) = item else {
                return None;
            };
            let name = obj.get("name").and_then(Value::as_str)?;
            let chart = obj.get("chart").and_then(Value::as_str)?;
            let (chart_name, chart_version) = split_chart(chart)?;
            Some(HelmRelease {
                name: name.to_owned(),
                chart_name: chart_name.to_owned(),
                chart_version: chart_version.to_owned(),
            })
        })
        .collect()
}

/// Splits a `helm list` chart like `nginx-ingress-1.2.3` into name and version.
///
/// The chart column is `<name>-<version>` where `name` may carry a repository
/// prefix such as `bitnami/`. The split happens at the last `-` whose right
/// hand side starts with a digit.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn split_chart(chart: &str) -> Option<(&str, &str)> {
    let (name, version) = chart.rsplit_once('-')?;
    if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        Some((name, version))
    } else {
        None
    }
}

/// Whether `candidate` is a newer version than `current`.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn version_newer(candidate: &str, current: &str) -> bool {
    version_key(candidate) > version_key(current)
}

/// A sortable key for a version string like `1.2.3`.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Whether the `helm` binary is discoverable on `PATH`.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_helm() -> Detection {
    if command_exists("helm") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use super::detect_helm;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use super::{parse_helm_list, split_chart, version_newer};
    use crate::step::Detection;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn detect_helm_reports_not_installed_when_absent_from_path() {
        // `helm` may be installed on the machine the test runs on (CI runners
        // ship it), so detection must be exercised against a `PATH` that does
        // not contain `helm` instead of relying on the ambient environment.
        let _guard = crate::process::PATH_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("PATH").ok();
        let dir = std::env::temp_dir().join(format!(
            "freshen-helm-absent-{pid}",
            pid = std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
            panic!("failed to create stub directory {}: {error}", dir.display())
        });
        std::env::set_var("PATH", &dir);
        assert!(
            !detect_helm().is_installed(),
            "helm must not be detected when absent from PATH"
        );
        if let Some(previous) = previous {
            std::env::set_var("PATH", previous);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_installed_is_not_installed() {
        assert!(!Detection::NotInstalled.is_installed());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn split_chart_separates_name_and_version() {
        assert_eq!(
            split_chart("nginx-ingress-1.2.3"),
            Some(("nginx-ingress", "1.2.3"))
        );
        assert_eq!(
            split_chart("bitnami/nginx-1.2.3"),
            Some(("bitnami/nginx", "1.2.3"))
        );
        assert_eq!(split_chart("mysql-8.0.35"), Some(("mysql", "8.0.35")));
        assert_eq!(split_chart("redis"), None);
        assert_eq!(split_chart(""), None);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn parse_helm_list_reads_chart_and_version() {
        let output = r#"[{"name":"my-release","namespace":"default","chart":"nginx-ingress-1.2.3","app_version":"1.2.3"}]"#;
        let releases = parse_helm_list(output);
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].name, "my-release");
        assert_eq!(releases[0].chart_name, "nginx-ingress");
        assert_eq!(releases[0].chart_version, "1.2.3");
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn parse_helm_list_ignores_invalid_json() {
        assert!(parse_helm_list("not json").is_empty());
        assert!(parse_helm_list("").is_empty());
        assert!(parse_helm_list("[]").is_empty());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn version_newer_compare_major_minor_patch() {
        assert!(version_newer("2.0.0", "1.2.3"));
        assert!(version_newer("1.2.4", "1.2.3"));
        assert!(!version_newer("1.2.3", "2.0.0"));
        assert!(!version_newer("1.2.3", "1.2.3"));
    }
}
