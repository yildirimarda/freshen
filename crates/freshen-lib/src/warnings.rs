//! Non-fatal warnings ported from `update-all.sh`.
//!
//! These situations deserve a warning, not a failure: the upgrade goes ahead,
//! but the user has to restart something afterwards or may lose work.
//!
//! * Upgrading zellij from inside a zellij session (or while a zellij server is
//!   running) leaves the running client unable to attach to sessions created
//!   by the old server.
//! * Upgrading tmux while its server is running leaves the old server binary in
//!   place, so new clients and existing sessions disagree on the version.
//! * Upgrading OpenSSH while SSH sessions are active keeps those sessions on
//!   the old server until they reconnect.
//! * Stopping Colima while a Docker build is running interrupts the build.
//! * `mise prune` cannot remove an old toolchain version while a process
//!   started by another application (an MCP server, for example) still runs
//!   from it. Node, Python, Java and Go are checked.

use std::env;
use std::path::{Path, PathBuf};

use crate::process::CommandSpec;

/// Warning shown before zellij is upgraded from inside a zellij session.
pub const ZELLIJ_WARNING: &str =
    "zellij will be upgraded while you are inside a zellij session. Close and recreate your sessions afterwards.";

/// Warning shown before zellij is upgraded while a server is running outside
/// the current session.
pub const ZELLIJ_SERVER_WARNING: &str =
    "zellij will be upgraded while a zellij server is running. Close and recreate your sessions afterwards.";

/// Warning shown before tmux is upgraded while a server is running.
pub const TMUX_WARNING: &str =
    "tmux will be upgraded while a tmux server is running. Restart the server and recreate your sessions afterwards.";

/// Warning shown before OpenSSH is upgraded while SSH sessions are active.
pub const SSH_WARNING: &str =
    "OpenSSH will be upgraded while SSH sessions are active. Existing sessions keep the old server; reconnect afterwards.";

/// Warning shown before Colima is stopped while Docker builds are running.
pub const DOCKER_BUILD_WARNING: &str =
    "Colima/lima is about to be upgraded while Docker builds are running. The running builds will be interrupted.";

/// Toolchains whose unused versions `mise prune` removes.
const TOOLCHAINS: [&str; 4] = ["node", "python", "java", "go"];

/// Header of the warning about processes running an old toolchain version.
///
/// The toolchain name is inserted so each group names the runtime the user
/// has to restart.
#[must_use]
pub fn stale_tool_warning(tool: &str) -> String {
    format!(
        "Processes still running on an old {tool} version (restart the app that started them so mise can prune it):"
    )
}

/// A process that keeps an old toolchain version alive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleToolProcess {
    /// Process id.
    pub pid: u32,
    /// Name of the application that started it, as reported by `ps`.
    pub parent: String,
    /// Full command line, truncated to 80 characters when printed.
    pub command: String,
}

impl StaleToolProcess {
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

fn inside_tmux_session() -> bool {
    env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

/// Warns when an upgrade of zellij, tmux or OpenSSH would break a running
/// interactive session.
///
/// The probe is best effort: a missing `ps` or an empty result still allows
/// the environment-based zellij and SSH checks to warn, and never fails the
/// step. Only sessions that belong to a package in `outdated_formulae` are
/// reported, so a run that upgrades something unrelated stays quiet.
pub fn warn_about_breakable_sessions(outdated_formulae: &str) {
    let inside_zellij = inside_zellij_session();
    let inside_tmux = inside_tmux_session();
    let over_ssh = ssh_environment_session();
    let zellij_outdated = is_outdated(outdated_formulae, &["zellij"]);
    let tmux_outdated = is_outdated(outdated_formulae, &["tmux"]);
    let ssh_outdated = is_outdated(
        outdated_formulae,
        &["openssh", "openssh-server", "dropbear"],
    );
    let needs_processes = (!inside_zellij && zellij_outdated)
        || (tmux_outdated && !inside_tmux)
        || (ssh_outdated && !over_ssh);
    let processes = if needs_processes {
        running_processes()
    } else {
        Vec::new()
    };

    if zellij_warning_applies(outdated_formulae, inside_zellij) {
        println!("warning: {ZELLIJ_WARNING}");
    } else if zellij_outdated
        && processes
            .iter()
            .any(|process| is_zellij_process(&process.command))
    {
        println!("warning: {ZELLIJ_SERVER_WARNING}");
    }

    if tmux_outdated
        && (inside_tmux
            || processes
                .iter()
                .any(|process| is_tmux_process(&process.command)))
    {
        println!("warning: {TMUX_WARNING}");
    }

    if ssh_outdated
        && (over_ssh
            || processes
                .iter()
                .any(|process| is_ssh_process(&process.command)))
    {
        println!("warning: {SSH_WARNING}");
    }
}

/// Warns when Docker builds are running and stopping Colima would interrupt
/// them.
///
/// Called once at the start of a run that includes a container step, so the
/// warning is printed before anything is stopped.
pub fn warn_about_running_docker_builds() {
    let builds: Vec<ProcessEntry> = running_processes()
        .into_iter()
        .filter(|process| is_docker_build(&process.command))
        .collect();
    if builds.is_empty() {
        return;
    }

    println!("warning: {DOCKER_BUILD_WARNING}");
    for process in builds {
        println!(
            "   - pid {}: {}",
            process.pid,
            truncate_chars(&process.command, 80)
        );
    }
}

fn is_outdated(outdated_formulae: &str, names: &[&str]) -> bool {
    outdated_formulae
        .lines()
        .any(|line| names.contains(&line.trim()))
}

fn ssh_environment_session() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|name| env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn running_processes() -> Vec<ProcessEntry> {
    let Ok(output) = CommandSpec::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .run()
    else {
        return Vec::new();
    };
    parse_process_table(output.stdout())
}

fn is_zellij_process(command: &str) -> bool {
    program_name(command) == "zellij"
}

fn is_tmux_process(command: &str) -> bool {
    program_name(command) == "tmux"
}

fn is_ssh_process(command: &str) -> bool {
    match program_name(command) {
        "ssh" => true,
        "sshd" | "sshd-session" => ["@pts", "@tty", "@notty", "sshd-session", "[priv]"]
            .iter()
            .any(|marker| command.contains(marker)),
        _ => false,
    }
}

fn is_docker_build(command: &str) -> bool {
    let mut arguments = command.split_whitespace().skip(1);
    match program_name(command) {
        "docker" | "podman" => {
            let Some(subcommand) = arguments.find(|argument| !argument.starts_with('-')) else {
                return false;
            };
            match subcommand {
                "build" => true,
                "buildx" | "compose" => arguments.any(|argument| argument == "build"),
                _ => false,
            }
        }
        "docker-compose" | "podman-compose" => arguments.any(|argument| argument == "build"),
        _ => false,
    }
}

fn program_name(command: &str) -> &str {
    let program = command.split_whitespace().next().unwrap_or("");
    basename(program).trim_end_matches(':')
}

/// Warns about processes still running from an old toolchain install under mise.
///
/// The probe is best effort: a missing `mise`, a failed `ps` or an empty result
/// prints nothing and never fails the step. Only toolchains that mise reports
/// an active version for are checked.
pub fn warn_about_stale_tool_processes() {
    let active_toolchains = active_toolchain_paths();
    if active_toolchains.is_empty() {
        return;
    }
    let processes = running_processes();

    for (tool, active_path) in active_toolchains {
        let entries = stale_tool_entries(&processes, &active_path);
        if entries.is_empty() {
            continue;
        }

        println!("warning: {}", stale_tool_warning(tool));
        for entry in entries {
            let process = StaleToolProcess {
                pid: entry.pid,
                parent: parent_name(entry.ppid),
                command: entry.command,
            };
            println!("{}", process.display_line());
        }
    }
}

fn active_toolchain_paths() -> Vec<(&'static str, PathBuf)> {
    TOOLCHAINS
        .iter()
        .filter_map(|&tool| active_tool_path(tool).map(|path| (tool, path)))
        .collect()
}

fn active_tool_path(tool: &str) -> Option<PathBuf> {
    let output = CommandSpec::new("mise").args(["where", tool]).run().ok()?;
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

fn stale_tool_entries(processes: &[ProcessEntry], active_tool: &Path) -> Vec<ProcessEntry> {
    let Some(install_dir) = active_tool.parent() else {
        return Vec::new();
    };
    let install_prefix = format!("{}/", install_dir.display());
    let active_prefix = format!("{}/", active_tool.display());

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
        basename, is_docker_build, is_outdated, is_ssh_process, is_tmux_process, is_zellij_process,
        parse_process_table, stale_tool_entries, stale_tool_warning, zellij_warning_applies,
        StaleToolProcess,
    };

    const ACTIVE_NODE: &str = "/home/user/.local/share/mise/installs/node/22.1.0";
    const ACTIVE_PYTHON: &str = "/home/user/.local/share/mise/installs/python/3.12.0";

    #[test]
    fn zellij_warning_needs_an_outdated_zellij_and_a_session() {
        assert!(zellij_warning_applies("zellij\nnode\n", true));
        assert!(zellij_warning_applies("  zellij  \n", true));
        assert!(!zellij_warning_applies("zellij\n", false));
        assert!(!zellij_warning_applies("zellij-utils\nnode\n", true));
        assert!(!zellij_warning_applies("", true));
    }

    #[test]
    fn only_outdated_packages_trigger_session_warnings() {
        assert!(is_outdated("tmux\nnode\n", &["tmux"]));
        assert!(is_outdated("  openssh  \n", &["openssh", "dropbear"]));
        assert!(!is_outdated("openssh-utils\n", &["openssh"]));
        assert!(!is_outdated("", &["tmux"]));
    }

    #[test]
    fn tmux_and_zellij_servers_are_detected_from_the_process_table() {
        assert!(is_tmux_process(
            "/opt/homebrew/bin/tmux new-session -d -s work"
        ));
        assert!(is_tmux_process("tmux: server"));
        assert!(!is_tmux_process("/usr/bin/git tmux"));

        assert!(is_zellij_process("zellij --server /tmp/zellij-1000"));
        assert!(!is_zellij_process("zellij-utils"));
    }

    #[test]
    fn ssh_sessions_are_detected_without_treating_the_daemon_as_a_session() {
        assert!(is_ssh_process("sshd: user@pts/0"));
        assert!(is_ssh_process("sshd-session: user [priv]"));
        assert!(is_ssh_process("ssh user@example.com"));
        assert!(!is_ssh_process("/usr/sbin/sshd -D"));
        assert!(!is_ssh_process("ssh-agent"));
    }

    #[test]
    fn docker_builds_are_detected_but_other_docker_commands_are_not() {
        assert!(is_docker_build("docker build -t app ."));
        assert!(is_docker_build("/usr/bin/docker buildx build --push ."));
        assert!(is_docker_build("docker compose build web"));
        assert!(is_docker_build("docker-compose build"));
        assert!(!is_docker_build("docker ps"));
        assert!(!is_docker_build("docker run --name build alpine"));
        assert!(!is_docker_build("docker compose up -d"));
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
    fn only_processes_from_an_older_install_of_the_same_toolchain_are_stale() {
        let table = format!(
            "\
  100 1 {ACTIVE_NODE}/bin/node /current.js
  101 1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node /old.js
  102 1 /usr/bin/node /system.js
  103 1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node-foo /old.js
  200 1 {ACTIVE_PYTHON}/bin/python3 /current.py
  201 1 /home/user/.local/share/mise/installs/python/3.10.0/bin/python3 /old.py
"
        );
        let processes = parse_process_table(&table);

        let stale = stale_tool_entries(&processes, Path::new(ACTIVE_NODE));

        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].pid, 101);
        assert_eq!(stale[1].pid, 103);

        let stale = stale_tool_entries(&processes, Path::new(ACTIVE_PYTHON));

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].pid, 201);
    }

    #[test]
    fn the_warning_names_the_toolchain() {
        assert_eq!(
            stale_tool_warning("python"),
            "Processes still running on an old python version (restart the app that started them so mise can prune it):"
        );
        assert_eq!(
            stale_tool_warning("go"),
            "Processes still running on an old go version (restart the app that started them so mise can prune it):"
        );
    }

    #[test]
    fn a_display_line_matches_the_zsh_format_and_truncates_at_eighty_characters() {
        let process = StaleToolProcess {
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
