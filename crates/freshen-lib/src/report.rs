//! JSON and Markdown renderings of run and check summaries.
//!
//! The plain-text summary is the default; these renderings are selected with
//! `--json` or `--markdown`. Every step appears in the report, including its
//! status, duration and the versions that actually changed while it ran.

use serde_json::{json, Value};

use crate::check::CheckSummary;
use crate::runner::{format_duration, RunSummary};

pub(crate) fn run_json(summary: &RunSummary) -> String {
    let steps: Vec<Value> = summary
        .steps()
        .iter()
        .map(|result| {
            let changes: Vec<Value> = result
                .changes()
                .iter()
                .map(|change| {
                    json!({
                        "name": change.name(),
                        "before": change.before(),
                        "after": change.after(),
                    })
                })
                .collect();

            let cleanup: Vec<Value> = result
                .cleanups()
                .iter()
                .map(|report| {
                    json!({
                        "removed": report.removed(),
                        "reclaimed": report.reclaimed(),
                    })
                })
                .collect();

            let doctor_warnings: Vec<Value> = result
                .doctor_warnings()
                .iter()
                .map(|finding| {
                    json!({
                        "message": finding.message,
                        "remedy": finding.remedy,
                    })
                })
                .collect();

            let bundle_cleanup_packages: Vec<Value> = result
                .bundle_cleanup_packages()
                .iter()
                .map(|package| {
                    json!({
                        "name": package.name,
                        "kind": package.kind,
                    })
                })
                .collect();

            json!({
                "id": result.id(),
                "name": result.name(),
                "status": result.status().as_str(),
                "duration_ms": result.duration().as_millis(),
                "changes": changes,
                "cleanup": cleanup,
                "doctor_warnings": doctor_warnings,
                "bundle_cleanup_packages": bundle_cleanup_packages,
                "skip_reason": result.skip_reason().map(|reason| reason.to_string()),
                "skipped_items": result.skipped_items().to_vec(),
                "restarted_services": result.restarted_services().to_vec(),
                "left_running_apps": result.left_running_apps().to_vec(),
                "reopened_hidden": result.reopened_hidden().to_vec(),
                "restored_windows": result.restored_windows().to_vec(),
                "error": result.error().map(|error| error.to_string()),
            })
        })
        .collect();

    let rollback: Vec<Value> = summary
        .rollback_commands()
        .iter()
        .map(|hint| {
            json!({
                "group": hint.group(),
                "command": hint.command(),
            })
        })
        .collect();

    let container_runtime = summary.container_runtime().map(|runtime| {
        json!({
            "name": runtime.kind().as_str(),
            "display_name": runtime.kind().display_name(),
            "source": runtime.source().to_string(),
        })
    });

    let report = json!({
        "container_runtime": container_runtime,
        "ok": summary.ok_count(),
        "planned": summary.planned_count(),
        "skipped": summary.skipped_count(),
        "failed": summary.failed_count(),
        "timed_out": summary.timed_out_count(),
        "interrupted": summary.interrupted_count(),
        "was_interrupted": summary.was_interrupted(),
        "restore_failures": summary.restore_failures(),
        "total_duration_ms": summary.total_duration().as_millis(),
        "snapshot": summary.snapshot_path().map(|path| path.display().to_string()),
        "rollback": rollback,
        "steps": steps,
    });

    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_owned())
}

pub(crate) fn run_markdown(summary: &RunSummary) -> String {
    let mut out = String::new();
    out.push_str("# freshen report\n\n");
    if let Some(runtime) = summary.container_runtime() {
        out.push_str(&format!("**Container runtime:** {runtime}\n\n"));
    }
    out.push_str(&format!(
        "**Summary:** {} ok, {} planned, {} skipped, {} failed, {} timed out, {} interrupted ({})\n\n",
        summary.ok_count(),
        summary.planned_count(),
        summary.skipped_count(),
        summary.failed_count(),
        summary.timed_out_count(),
        summary.interrupted_count(),
        format_duration(summary.total_duration())
    ));
    out.push_str("| Step | Status | Duration | Changes |\n");
    out.push_str("| --- | --- | --- | --- |\n");

    for result in summary.steps() {
        let mut status = result.status().to_string();
        if let Some(reason) = result.skip_reason() {
            status.push_str(" - ");
            status.push_str(&reason.to_string());
        }
        if let Some(error) = result.error() {
            status.push_str(" - ");
            status.push_str(&error.to_string());
        }

        let changes = if result.changes().is_empty() {
            "none".to_owned()
        } else {
            result
                .changes()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("<br>")
        };

        out.push_str(&format!(
            "| {} (`{}`) | {} | {} | {} |\n",
            escape_cell(result.name()),
            result.id(),
            escape_cell(&status),
            format_duration(result.duration()),
            escape_cell(&changes)
        ));
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.cleanups().is_empty())
    {
        out.push_str("\n## Cleanup\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.cleanups().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for report in result.cleanups() {
                match report.reclaimed() {
                    Some(space) => out.push_str(&format!("Reclaimed space: `{space}`\n\n")),
                    None => out.push_str("Reclaimed space: not reported by the tool\n\n"),
                }
                for line in report.removed() {
                    out.push_str("- ");
                    out.push_str(line);
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.doctor_warnings().is_empty())
    {
        out.push_str("\n## Doctor warnings\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.doctor_warnings().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for warning in result.doctor_warnings() {
                out.push_str("- ");
                out.push_str(&warning.message);
                if let Some(remedy) = &warning.remedy {
                    out.push_str(&format!(" ({remedy})"));
                }
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.bundle_cleanup_packages().is_empty())
    {
        out.push_str("\n## Bundle cleanup packages\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.bundle_cleanup_packages().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for package in result.bundle_cleanup_packages() {
                out.push_str(&format!("- {} ({})\n", package.name, package.kind));
            }
            out.push('\n');
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.skipped_items().is_empty())
    {
        out.push_str("\n## Skipped items\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.skipped_items().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for item in result.skipped_items() {
                out.push_str("- ");
                out.push_str(item);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.restarted_services().is_empty())
    {
        out.push_str("\n## Restarted services\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.restarted_services().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for service in result.restarted_services() {
                out.push_str("- ");
                out.push_str(service);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.left_running_apps().is_empty())
    {
        out.push_str("\n## Left running apps\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.left_running_apps().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for app in result.left_running_apps() {
                out.push_str("- ");
                out.push_str(app);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.restored_windows().is_empty())
    {
        out.push_str("\n## Restored windows\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.restored_windows().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for app in result.restored_windows() {
                out.push_str("- ");
                out.push_str(app);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if summary
        .steps()
        .iter()
        .any(|result| !result.reopened_hidden().is_empty())
    {
        out.push_str("\n## Reopened hidden apps\n\n");
        for result in summary
            .steps()
            .iter()
            .filter(|result| !result.reopened_hidden().is_empty())
        {
            out.push_str(&format!(
                "### {} (`{}`)\n\n",
                escape_cell(result.name()),
                result.id()
            ));
            for app in result.reopened_hidden() {
                out.push_str("- ");
                out.push_str(app);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if !summary.restore_failures().is_empty() {
        out.push_str("\n## Restore warnings\n\n");
        for failure in summary.restore_failures() {
            out.push_str("- ");
            out.push_str(failure);
            out.push('\n');
        }
    }

    if !summary.rollback_commands().is_empty() {
        out.push_str("\n## Rollback hints\n\n");
        if let Some(path) = summary.snapshot_path() {
            out.push_str(&format!("Snapshot: `{}`\n\n", path.display()));
        }
        out.push_str("Run these commands to return to the versions recorded before the run:\n\n");
        out.push_str("```sh\n");
        for hint in summary.rollback_commands() {
            out.push_str(hint.command());
            out.push('\n');
        }
        out.push_str("```\n");
    }

    out
}

pub(crate) fn check_json(summary: &CheckSummary) -> String {
    let steps: Vec<Value> = summary
        .steps()
        .iter()
        .map(|result| {
            let items: Vec<Value> = result
                .report()
                .items()
                .iter()
                .map(|item| {
                    json!({
                        "name": item.name(),
                        "current": item.current(),
                        "latest": item.latest(),
                        "reason": item.reason(),
                    })
                })
                .collect();

            json!({
                "id": result.id(),
                "name": result.name(),
                "status": result.status().as_str(),
                "duration_ms": result.duration().as_millis(),
                "pending": result.report().pending(),
                "items": items,
                "skip_reason": result.skip_reason().map(|reason| reason.to_string()),
                "error": result.error().map(|error| error.to_string()),
            })
        })
        .collect();

    let report = json!({
        "up_to_date": summary.up_to_date_count(),
        "outdated": summary.outdated_steps(),
        "skipped": summary.skipped_count(),
        "failed": summary.failed_count(),
        "timed_out": summary.timed_out_count(),
        "pending": summary.pending(),
        "steps": steps,
    });

    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_owned())
}

pub(crate) fn check_markdown(summary: &CheckSummary) -> String {
    let mut out = String::new();
    out.push_str("# freshen check\n\n");
    out.push_str(&format!(
        "**Summary:** {} up to date, {} outdated, {} skipped, {} failed, {} timed out ({} update(s) pending)\n\n",
        summary.up_to_date_count(),
        summary.outdated_steps(),
        summary.skipped_count(),
        summary.failed_count(),
        summary.timed_out_count(),
        summary.pending()
    ));
    out.push_str("| Step | Status | Duration | Outdated |\n");
    out.push_str("| --- | --- | --- | --- |\n");

    for result in summary.steps() {
        let mut status = result.status().to_string();
        if let Some(reason) = result.skip_reason() {
            status.push_str(" - ");
            status.push_str(&reason.to_string());
        }
        if let Some(error) = result.error() {
            status.push_str(" - ");
            status.push_str(&error.to_string());
        }

        let items = if result.report().items().is_empty() {
            "none".to_owned()
        } else {
            result
                .report()
                .items()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("<br>")
        };

        out.push_str(&format!(
            "| {} (`{}`) | {} | {} | {} |\n",
            escape_cell(result.name()),
            result.id(),
            escape_cell(&status),
            format_duration(result.duration()),
            escape_cell(&items)
        ));
    }

    out
}

fn escape_cell(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br>")
}

#[cfg(test)]
mod tests {
    use super::{check_json, check_markdown, run_json, run_markdown};
    use crate::check::{CheckStatus, CheckSummary};
    use crate::options::RunOptions;
    use crate::platform::Platform;
    use crate::runner::Runner;
    use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ReportStep {
        report: CheckReport,
        after: CheckReport,
        ran: AtomicBool,
    }

    impl ReportStep {
        fn new(before: CheckReport, after: CheckReport) -> Self {
            Self {
                report: before,
                after,
                ran: AtomicBool::new(false),
            }
        }
    }

    impl Step for ReportStep {
        fn id(&self) -> StepId {
            "brew-formulae"
        }

        fn display_name(&self) -> &str {
            "Homebrew formulae"
        }

        fn detect(&self) -> Result<Detection, StepError> {
            Ok(Detection::Installed)
        }

        fn check(&self) -> Result<CheckReport, StepError> {
            if self.ran.load(Ordering::SeqCst) {
                Ok(self.after.clone())
            } else {
                Ok(self.report.clone())
            }
        }

        fn run(&self) -> Result<(), StepError> {
            self.ran.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn platforms(&self) -> &'static [Platform] {
            &Platform::ALL
        }
    }

    fn summary() -> crate::runner::RunSummary {
        let step = ReportStep::new(
            CheckReport::from_items([OutdatedItem::new("jq")
                .with_current("1.7.1")
                .with_latest("1.8.0")]),
            CheckReport::up_to_date(),
        );
        Runner::with_options([&step as &dyn Step], RunOptions::new()).run()
    }

    #[test]
    fn run_json_contains_the_step_and_its_version_changes() {
        let text = run_json(&summary());
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => panic!("invalid JSON report: {error}: {text}"),
        };

        assert_eq!(value["ok"], 1);
        assert_eq!(value["steps"][0]["id"], "brew-formulae");
        assert_eq!(value["steps"][0]["changes"][0]["name"], "jq");
        assert_eq!(value["steps"][0]["changes"][0]["before"], "1.7.1");
        assert_eq!(value["steps"][0]["changes"][0]["after"], "1.8.0");
    }

    #[test]
    fn run_markdown_contains_a_table_row_with_the_version_changes() {
        let text = run_markdown(&summary());

        assert!(text.starts_with("# freshen report\n"));
        assert!(text.contains("| Step | Status | Duration | Changes |"));
        assert!(text.contains("Homebrew formulae"));
        assert!(text.contains("jq 1.7.1 -> 1.8.0"));
    }

    #[test]
    fn a_step_without_version_changes_says_none_in_markdown() {
        let step = ReportStep::new(CheckReport::up_to_date(), CheckReport::up_to_date());
        let summary = Runner::with_options([&step as &dyn Step], RunOptions::new()).run();

        let text = run_markdown(&summary);
        assert!(
            text.lines()
                .any(|line| line.contains("| ok |") && line.ends_with("| none |")),
            "markdown: {text}"
        );
    }

    #[test]
    fn check_reports_include_the_outdated_items() {
        let step = ReportStep::new(
            CheckReport::from_items([OutdatedItem::new("jq")
                .with_current("1.7.1")
                .with_latest("1.8.0")]),
            CheckReport::up_to_date(),
        );
        let summary: CheckSummary =
            Runner::with_options([&step as &dyn Step], RunOptions::new()).check();

        let json = check_json(&summary);
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(value) => value,
            Err(error) => panic!("invalid JSON report: {error}: {json}"),
        };
        assert_eq!(value["outdated"], 1);
        assert_eq!(value["steps"][0]["items"][0]["name"], "jq");

        let markdown = check_markdown(&summary);
        assert!(markdown.contains("# freshen check"));
        assert!(markdown.contains("jq 1.7.1 -> 1.8.0"));
    }

    #[test]
    fn check_reports_include_the_reason_an_item_needs_greedy() {
        let step = ReportStep::new(
            CheckReport::from_items([OutdatedItem::new("nightly")
                .with_current("1.0")
                .with_latest("latest")
                .with_reason("version :latest; needs --greedy")]),
            CheckReport::up_to_date(),
        );
        let summary: CheckSummary =
            Runner::with_options([&step as &dyn Step], RunOptions::new()).check();

        let value: serde_json::Value = match serde_json::from_str(&check_json(&summary)) {
            Ok(value) => value,
            Err(error) => panic!("invalid JSON report: {error}"),
        };
        assert_eq!(
            value["steps"][0]["items"][0]["reason"],
            "version :latest; needs --greedy"
        );

        let markdown = check_markdown(&summary);
        assert!(
            markdown.contains("nightly 1.0 -> latest (version :latest; needs --greedy)"),
            "markdown: {markdown}"
        );
    }

    #[test]
    fn check_status_is_used_in_the_json_report() {
        let step = ReportStep::new(CheckReport::up_to_date(), CheckReport::up_to_date());
        let summary = Runner::with_options([&step as &dyn Step], RunOptions::new()).check();

        let value: serde_json::Value = match serde_json::from_str(&check_json(&summary)) {
            Ok(value) => value,
            Err(error) => panic!("invalid JSON report: {error}"),
        };
        assert_eq!(value["steps"][0]["status"], CheckStatus::UpToDate.as_str());
    }
}
