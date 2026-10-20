//! Non-fatal warnings ported from `update-all.sh`.
//!
//! Both situations here deserve a warning, not a failure: the upgrade goes
//! ahead, but the user has to restart something afterwards for the cleanup to
//! take effect.
//!
//! * Upgrading zellij from inside a zellij session leaves the running client
//!   unable to attach to sessions created by the old server.
//! * `mise prune` cannot remove an old Node version while a process started by
//!   another application (an MCP server, for example) still runs from it.

use std::env;
use std::path::{Path, PathBuf};

use crate::process::CommandSpec;

/// Warning shown before zellij is upgraded from inside a zellij session.
pub const ZELLIJ_WARNING: &str =
    "zellij will be upgraded while you are inside a zellij session. Close and recreate your sessions afterwards.";

/// Header of the warning about processes running an old Node version.
pub const STALE_NODE_WARNING: &str = "Processes still running on an old node version (restart the app that started them so mise can prune it):";

/// A process that keeps an old Node version alive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleNodeProcess {
    /// Process id.
    pub pid: u32,
    /// Name of the application that started it, as reported by `ps`.
    pub parent: String,
    /// Full command line, truncated to 80 characters when printed.
    pub command: String,
}

impl StaleNodeProcess {
    /// The indented line the zsh script prints for this process.
    #[must_use]
    pub fn display_line(&self) -> String {
        format!(
            "   - pid {} (started by: {}): {}",
            self.pid,
            self.parent,
            truncate_chars(&self.command, 80)
        )
    }
}

/// Warns when zellij is about to be upgraded from inside a zellij session.
pub fn warn_if_zellij_upgrade(outdated_formulae: &str) {
    if zellij_warning_applies(outdated_formulae, inside_zellij_session()) {
        println!("warning: {ZELLIJ_WARNING}");
    }
}

/// Whether the zellij warning applies to an outdated-formula listing.
#[must_use]
pub fn zellij_warning_applies(outdated_formulae: &str, inside_zellij_session: bool) -> bool {
    inside_zellij_session
        && outdated_formulae
            .lines()
            .any(|line| line.trim() == "zellij")
}

fn inside_zellij_session() -> bool {
    env::var_os("ZELLIJ").is_some_and(|value| !value.is_empty())
}

/// Warns about processes still running from an old Node install under mise.
///
/// The probe is best effort, matching the zsh script: a missing `mise`, a
/// failed `ps` or an empty result prints nothing and never fails the step.
pub fn warn_about_stale_node_processes() {
    let processes = stale_node_processes();
    if processes.is_empty() {
        return;
    }

    println!("warning: {STALE_NODE_WARNING}");
    for process in processes {
        println!("{}", process.display_line());
    }
}

fn stale_node_processes() -> Vec<StaleNodeProcess> {
    let Some(active_node) = active_node_path() else {
        return Vec::new();
    };
    let Ok(output) = CommandSpec::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .run()
    else {
        return Vec::new();
    };

    stale_node_entries(&parse_process_table(output.stdout()), &active_node)
        .into_iter()
        .map(|entry| StaleNodeProcess {
            pid: entry.pid,
            parent: parent_name(entry.ppid),
            command: entry.command,
        })
        .collect()
}

fn active_node_path() -> Option<PathBuf> {
    let output = CommandSpec::new("mise")
        .args(["where", "node"])
        .run()
        .ok()?;
    let path = output.stdout().trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessEntry {
    pid: u32,
    ppid: u32,
    command: String,
}

fn stale_node_entries(processes: &[ProcessEntry], active_node: &Path) -> Vec<ProcessEntry> {
    let Some(install_dir) = active_node.parent() else {
        return Vec::new();
    };
    let install_prefix = format!("{}/", install_dir.display());
    let active_prefix = format!("{}/", active_node.display());

    processes
        .iter()
        .filter(|process| {
            process.command.contains(&install_prefix) && !process.command.contains(&active_prefix)
        })
        .cloned()
        .collect()
}

fn parent_name(ppid: u32) -> String {
    CommandSpec::new("ps")
        .args(["-o", "comm=", "-p"])
        .arg(ppid.to_string())
        .run()
        .ok()
        .map(|output| basename(output.stdout().trim()).to_owned())
        .unwrap_or_default()
}

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

fn parse_process_table(table: &str) -> Vec<ProcessEntry> {
    table.lines().filter_map(parse_process_line).collect()
}

fn parse_process_line(line: &str) -> Option<ProcessEntry> {
    let rest = line.trim_start();
    let pid_end = rest.find(char::is_whitespace)?;
    let pid = rest.get(..pid_end)?.parse().ok()?;
    let rest = rest.get(pid_end..)?.trim_start();
    let ppid_end = rest.find(char::is_whitespace)?;
    let ppid = rest.get(..ppid_end)?.parse().ok()?;
    let command = rest.get(ppid_end..)?.trim_start().to_owned();

    Some(ProcessEntry { pid, ppid, command })
}

fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        basename, parse_process_table, stale_node_entries, zellij_warning_applies, StaleNodeProcess,
    };

    const ACTIVE_NODE: &str = "/home/user/.local/share/mise/installs/node/22.1.0";

    #[test]
    fn zellij_warning_needs_an_outdated_zellij_and_a_session() {
        assert!(zellij_warning_applies("zellij\nnode\n", true));
        assert!(zellij_warning_applies("  zellij  \n", true));
        assert!(!zellij_warning_applies("zellij\n", false));
        assert!(!zellij_warning_applies("zellij-utils\nnode\n", true));
        assert!(!zellij_warning_applies("", true));
    }

    #[test]
    fn process_table_keeps_spaces_in_the_command_line() {
        let table = "\
  100     1 /old/node /path/with spaces/server.js
  101   100 /new/node /current.js
";

        let processes = parse_process_table(table);

        assert_eq!(processes.len(), 2);
        assert_eq!(processes[0].pid, 100);
        assert_eq!(processes[0].ppid, 1);
        assert_eq!(
            processes[0].command,
            "/old/node /path/with spaces/server.js"
        );
        assert_eq!(processes[1].command, "/new/node /current.js");
    }

    #[test]
    fn malformed_process_lines_are_ignored() {
        let table = "\
not a process
  200 2
  201   3 /bin/tool
";

        let processes = parse_process_table(table);

        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].pid, 201);
    }

    #[test]
    fn only_processes_from_an_older_node_install_are_stale() {
        let table = format!(
            "\
  100 1 {ACTIVE_NODE}/bin/node /current.js
  101 1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node /old.js
  102 1 /usr/bin/node /system.js
  103 1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node-foo /old.js
"
        );
        let processes = parse_process_table(&table);

        let stale = stale_node_entries(&processes, Path::new(ACTIVE_NODE));

        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].pid, 101);
        assert_eq!(stale[1].pid, 103);
    }

    #[test]
    fn a_display_line_matches_the_zsh_format_and_truncates_at_eighty_characters() {
        let process = StaleNodeProcess {
            pid: 4242,
            parent: "LM Studio".to_owned(),
            command: "x".repeat(120),
        };

        let line = process.display_line();

        assert!(line.starts_with("   - pid 4242 (started by: LM Studio): "));
        assert_eq!(line.matches('x').count(), 80);
    }

    #[test]
    fn parent_names_use_the_trailing_path_component() {
        assert_eq!(
            basename("/Applications/LM Studio.app/Contents/MacOS/LM Studio"),
            "LM Studio"
        );
        assert_eq!(basename("zsh"), "zsh");
        assert_eq!(basename(""), "");
    }
}
