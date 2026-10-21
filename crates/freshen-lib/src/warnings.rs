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
//! * Upgrading GNU screen, byobu, dtach, abduco or mosh while one of its
//!   sessions is running leaves the running session on the old binary.
//! * Stopping Colima while a Docker build is running interrupts the build.
//! * `mise prune` cannot remove an old toolchain version while a process
//!   started by another application (an MCP server, for example) still runs
//!   from it. Only versions that `mise ls --prunable` lists are reported, so a
//!   version kept by a config is not mistaken for an old one. Node, Python,
//!   Java and Go are checked.

use std::env;
use std::path::{Path, PathBuf};

use crate::output::human;
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

/// Warning shown before GNU screen is upgraded while sessions are running.
pub const SCREEN_WARNING: &str =
    "GNU screen will be upgraded while screen sessions are running. Restart the server and recreate your sessions afterwards.";

/// Warning shown before byobu is upgraded while a session is running.
pub const BYOBU_WARNING: &str =
    "byobu will be upgraded while a byobu session is running. Close and recreate your sessions afterwards.";

/// Warning shown before dtach is upgraded while sessions are running.
pub const DTACH_WARNING: &str =
    "dtach will be upgraded while dtach sessions are running. Restart the sessions afterwards.";

/// Warning shown before abduco is upgraded while sessions are running.
pub const ABDUCO_WARNING: &str =
    "abduco will be upgraded while abduco sessions are running. Restart the sessions afterwards.";

/// Warning shown before mosh is upgraded while mosh-server sessions are running.
pub const MOSH_WARNING: &str =
    "mosh will be upgraded while mosh-server sessions are running. Reconnect afterwards.";

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

/// A tool whose upgrade can break a running interactive session.
///
/// Adding a session-aware tool is one row in [`SESSION_GUARDS`]: the package
/// names that trigger the warning, the environment variables that identify a
/// session of that tool, and how to recognise its server process.
#[derive(Debug, Clone, Copy)]
struct SessionGuard {
    /// Display name used by the README and the supported-tool list.
    name: &'static str,
    /// Outdated package names that make this warning relevant.
    packages: &'static [&'static str],
    /// Environment variables that are set inside a session of this tool.
    environment: &'static [&'static str],
    /// Whether a process command line belongs to this tool's server.
    is_server: fn(&str) -> bool,
    /// Warning shown when the upgrade happens inside a session.
    inside_warning: &'static str,
    /// Warning shown when a server runs outside the current session.
    server_warning: &'static str,
}

impl SessionGuard {
    fn is_outdated(self, outdated_formulae: &str) -> bool {
        is_outdated(outdated_formulae, self.packages)
    }

    fn inside_session(self) -> bool {
        self.environment
            .iter()
            .any(|name| environment_variable_set(name))
    }

    fn is_server_process(self, command: &str) -> bool {
        (self.is_server)(command)
    }
}

/// The tools checked before an upgrade for sessions it would break.
const SESSION_GUARDS: &[SessionGuard] = &[
    SessionGuard {
        name: "zellij",
        packages: &["zellij"],
        environment: &["ZELLIJ"],
        is_server: is_zellij_process,
        inside_warning: ZELLIJ_WARNING,
        server_warning: ZELLIJ_SERVER_WARNING,
    },
    SessionGuard {
        name: "tmux",
        packages: &["tmux"],
        environment: &["TMUX"],
        is_server: is_tmux_process,
        inside_warning: TMUX_WARNING,
        server_warning: TMUX_WARNING,
    },
    SessionGuard {
        name: "OpenSSH",
        packages: &["openssh", "openssh-server", "dropbear"],
        environment: &["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"],
        is_server: is_ssh_process,
        inside_warning: SSH_WARNING,
        server_warning: SSH_WARNING,
    },
    SessionGuard {
        name: "GNU screen",
        packages: &["screen", "gnu-screen"],
        environment: &["STY"],
        is_server: is_screen_process,
        inside_warning: SCREEN_WARNING,
        server_warning: SCREEN_WARNING,
    },
    SessionGuard {
        name: "byobu",
        packages: &["byobu"],
        environment: &["BYOBU_CONFIG_DIR", "BYOBU_BACKEND", "BYOBU_PREFIX"],
        is_server: is_byobu_process,
        inside_warning: BYOBU_WARNING,
        server_warning: BYOBU_WARNING,
    },
    SessionGuard {
        name: "dtach",
        packages: &["dtach"],
        environment: &[],
        is_server: is_dtach_process,
        inside_warning: DTACH_WARNING,
        server_warning: DTACH_WARNING,
    },
    SessionGuard {
        name: "abduco",
        packages: &["abduco"],
        environment: &["ABDUCO_SESSION", "ABDUCO_SOCKET"],
        is_server: is_abduco_process,
        inside_warning: ABDUCO_WARNING,
        server_warning: ABDUCO_WARNING,
    },
    SessionGuard {
        name: "mosh",
        packages: &["mosh"],
        environment: &[],
        is_server: is_mosh_process,
        inside_warning: MOSH_WARNING,
        server_warning: MOSH_WARNING,
    },
];

/// Session-aware tools freshen warns about before an upgrade, in
/// documentation order.
#[must_use]
pub fn supported_session_managers() -> Vec<&'static str> {
    SESSION_GUARDS.iter().map(|guard| guard.name).collect()
}

/// Warnings that apply to an outdated-formula listing, a per-guard
/// in-session flag and a snapshot of the process table.
fn session_warnings(
    outdated_formulae: &str,
    inside_sessions: &[bool],
    processes: &[ProcessEntry],
) -> Vec<&'static str> {
    SESSION_GUARDS
        .iter()
        .zip(inside_sessions)
        .filter_map(|(guard, inside_session)| {
            if !guard.is_outdated(outdated_formulae) {
                return None;
            }
            if *inside_session {
                return Some(guard.inside_warning);
            }
            processes
                .iter()
                .any(|process| guard.is_server_process(&process.command))
                .then_some(guard.server_warning)
        })
        .collect()
}

fn environment_variable_set(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// Warns when an upgrade of zellij, tmux or OpenSSH would break a running
/// interactive session.
///
/// The probe is best effort: a missing `ps` or an empty result still allows
/// the environment-based zellij and SSH checks to warn, and never fails the
/// step. Only sessions that belong to a package in `outdated_formulae` are
/// reported, so a run that upgrades something unrelated stays quiet.
pub fn warn_about_breakable_sessions(outdated_formulae: &str) {
    let inside_sessions: Vec<bool> = SESSION_GUARDS
        .iter()
        .map(|guard| guard.inside_session())
        .collect();
    let needs_processes = SESSION_GUARDS
        .iter()
        .zip(&inside_sessions)
        .any(|(guard, inside_session)| guard.is_outdated(outdated_formulae) && !inside_session);
    let processes = if needs_processes {
        running_processes()
    } else {
        Vec::new()
    };

    for warning in session_warnings(outdated_formulae, &inside_sessions, &processes) {
        human!("warning: {warning}");
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

    human!("warning: {DOCKER_BUILD_WARNING}");
    for process in builds {
        human!(
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

fn running_processes() -> Vec<ProcessEntry> {
    HostProcessProbe.processes()
}

/// Source of the process facts the warnings use.
///
/// The host reads the real process table; tests inject a fixed one so they
/// never depend on the machine they run on.
trait ProcessProbe {
    /// A snapshot of the host process table.
    fn processes(&self) -> Vec<ProcessEntry>;

    /// Name of the process with `pid`, when the table contains it.
    fn parent_name(&self, pid: u32, processes: &[ProcessEntry]) -> String {
        processes
            .iter()
            .find(|process| process.pid == pid)
            .map(|process| program_name(&process.command).to_owned())
            .unwrap_or_default()
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct HostProcessProbe;

impl ProcessProbe for HostProcessProbe {
    fn processes(&self) -> Vec<ProcessEntry> {
        #[cfg(unix)]
        {
            ps_processes()
        }
        #[cfg(windows)]
        {
            windows_processes()
        }
        #[cfg(not(any(unix, windows)))]
        {
            Vec::new()
        }
    }

    #[cfg(unix)]
    fn parent_name(&self, pid: u32, _processes: &[ProcessEntry]) -> String {
        ps_parent_name(pid)
    }
}

#[cfg(unix)]
fn ps_processes() -> Vec<ProcessEntry> {
    let Ok(output) = CommandSpec::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .run()
    else {
        return Vec::new();
    };
    parse_process_table(output.stdout())
}

#[cfg(unix)]
fn ps_parent_name(pid: u32) -> String {
    CommandSpec::new("ps")
        .args(["-o", "comm=", "-p"])
        .arg(pid.to_string())
        .run()
        .ok()
        .map(|output| basename(output.stdout().trim()).to_owned())
        .unwrap_or_default()
}

/// Reads the process table through CIM on Windows.
///
/// `CommandLine` is the only CIM field that keeps the full command line, which
/// stale-toolchain detection needs to match the install directory. Fields are
/// separated by the ASCII unit separator so spaces and tabs in a command line
/// stay intact.
#[cfg(windows)]
fn windows_processes() -> Vec<ProcessEntry> {
    let script = "Get-CimInstance Win32_Process | ForEach-Object { \
                  \"$($_.ProcessId)$([char]31)$($_.ParentProcessId)$([char]31)$($_.CommandLine)\" }";
    let Ok(output) = CommandSpec::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .run()
    else {
        return Vec::new();
    };
    parse_windows_process_table(output.stdout())
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

fn is_screen_process(command: &str) -> bool {
    program_name(command).eq_ignore_ascii_case("screen")
}

fn is_byobu_process(command: &str) -> bool {
    let program = program_name(command);
    program == "byobu" || program.starts_with("byobu-") || command.contains("byobu")
}

fn is_dtach_process(command: &str) -> bool {
    program_name(command) == "dtach"
}

fn is_abduco_process(command: &str) -> bool {
    program_name(command) == "abduco"
}

fn is_mosh_process(command: &str) -> bool {
    program_name(command) == "mosh-server"
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
    let command = command.trim_start();
    let program = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split('"').next().unwrap_or("")
    } else {
        command.split_whitespace().next().unwrap_or("")
    };
    let name = basename(program).trim_end_matches(':');
    name.strip_suffix(".exe").unwrap_or(name)
}

/// An installed tool version that `mise prune` will remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunableTool {
    /// Tool name, for example `node`.
    pub tool: String,
    /// Directory the version is installed in.
    pub path: PathBuf,
}

/// Lists the tool versions `mise prune` would remove.
///
/// `mise ls --prunable --json` is the same determination `mise prune` uses, so
/// a version that a config still needs is not listed even when it is not the
/// active version. Best effort: a missing `mise` or unexpected output yields
/// an empty list and never fails the step.
pub fn prunable_tool_versions() -> Vec<PrunableTool> {
    let Ok(output) = CommandSpec::new("mise")
        .args(["ls", "--prunable", "--json"])
        .run()
    else {
        return Vec::new();
    };

    parse_prunable_tool_versions(output.stdout())
}

fn parse_prunable_tool_versions(json: &str) -> Vec<PrunableTool> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(tools) = value.as_object() else {
        return Vec::new();
    };

    let mut prunable = Vec::new();
    for (tool, versions) in tools {
        if !TOOLCHAINS.contains(&tool.as_str()) {
            continue;
        }
        let Some(versions) = versions.as_array() else {
            continue;
        };
        for version in versions {
            let Some(path) = version.get("install_path").and_then(|path| path.as_str()) else {
                continue;
            };
            if path.is_empty() {
                continue;
            }
            prunable.push(PrunableTool {
                tool: tool.clone(),
                path: PathBuf::from(path),
            });
        }
    }
    prunable
}

/// Warns about processes still running from a toolchain version mise will prune.
///
/// The caller passes the versions from [`prunable_tool_versions`] because the
/// listing has to be taken before `mise prune` removes them. The probe is best
/// effort: a failed process-table query or an empty result prints nothing and
/// never fails the step. Windows reads the table through CIM; Unix uses `ps`.
pub fn warn_about_stale_tool_processes(prunable: &[PrunableTool]) {
    warn_about_stale_tool_processes_with(&HostProcessProbe, prunable);
}

fn warn_about_stale_tool_processes_with(probe: &dyn ProcessProbe, prunable: &[PrunableTool]) {
    if prunable.is_empty() {
        return;
    }
    let processes = probe.processes();

    for tool in TOOLCHAINS {
        let paths: Vec<&Path> = prunable
            .iter()
            .filter(|version| version.tool == tool)
            .map(|version| version.path.as_path())
            .collect();
        if paths.is_empty() {
            continue;
        }

        let entries = stale_tool_entries(&processes, &paths);
        if entries.is_empty() {
            continue;
        }

        human!("warning: {}", stale_tool_warning(tool));
        for entry in entries {
            let process = StaleToolProcess {
                pid: entry.pid,
                parent: probe.parent_name(entry.ppid, &processes),
                command: entry.command,
            };
            human!("{}", process.display_line());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessEntry {
    pid: u32,
    ppid: u32,
    command: String,
}

fn stale_tool_entries(processes: &[ProcessEntry], prunable_paths: &[&Path]) -> Vec<ProcessEntry> {
    let prefixes: Vec<String> = prunable_paths
        .iter()
        .map(|path| format!("{}/", normalized_path(path)))
        .collect();
    if prefixes.is_empty() {
        return Vec::new();
    }

    processes
        .iter()
        .filter(|process| {
            let command = normalize_path(&process.command);
            prefixes.iter().any(|prefix| command.contains(prefix))
        })
        .cloned()
        .collect()
}

fn normalized_path(path: &Path) -> String {
    normalize_path(&path.to_string_lossy())
        .trim_end_matches('/')
        .to_owned()
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn basename(path: &str) -> &str {
    let path = path.trim_matches('"');
    path.rsplit(['/', '\\']).next().unwrap_or(path)
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

#[cfg(any(test, windows))]
fn parse_windows_process_table(table: &str) -> Vec<ProcessEntry> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim_end_matches('\r').split('\u{1f}');
            let pid = fields.next()?.trim().parse().ok()?;
            let ppid = fields.next()?.trim().parse().ok()?;
            let command = fields.next().unwrap_or_default().trim();
            if command.is_empty() {
                return None;
            }
            Some(ProcessEntry {
                pid,
                ppid,
                command: command.to_owned(),
            })
        })
        .collect()
}

fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        basename, is_abduco_process, is_byobu_process, is_docker_build, is_dtach_process,
        is_mosh_process, is_outdated, is_screen_process, is_ssh_process, is_tmux_process,
        is_zellij_process, parse_process_table, parse_prunable_tool_versions,
        parse_windows_process_table, program_name, session_warnings, stale_tool_entries,
        stale_tool_warning, ProcessEntry, ProcessProbe, PrunableTool, StaleToolProcess,
        ABDUCO_WARNING, BYOBU_WARNING, DTACH_WARNING, MOSH_WARNING, SCREEN_WARNING, SESSION_GUARDS,
        SSH_WARNING, TMUX_WARNING, ZELLIJ_SERVER_WARNING, ZELLIJ_WARNING,
    };

    const ACTIVE_NODE: &str = "/home/user/.local/share/mise/installs/node/22.1.0";
    const ACTIVE_PYTHON: &str = "/home/user/.local/share/mise/installs/python/3.12.0";

    fn process(pid: u32, command: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid: 1,
            command: command.to_owned(),
        }
    }

    #[test]
    fn every_session_guard_warns_inside_its_own_session() {
        let inside = [true; SESSION_GUARDS.len()];

        assert_eq!(
            session_warnings(
                "zellij\ntmux\nopenssh\nscreen\nbyobu\ndtach\nabduco\nmosh\n",
                &inside,
                &[]
            ),
            [
                ZELLIJ_WARNING,
                TMUX_WARNING,
                SSH_WARNING,
                SCREEN_WARNING,
                BYOBU_WARNING,
                DTACH_WARNING,
                ABDUCO_WARNING,
                MOSH_WARNING,
            ]
        );
    }

    #[test]
    fn every_session_guard_warns_about_a_running_server() {
        let processes = [
            process(1, "zellij --server /tmp/zellij-1000"),
            process(2, "/opt/homebrew/bin/tmux new-session -d -s work"),
            process(3, "sshd: user@pts/0"),
            process(4, "SCREEN -S work"),
            process(5, "/usr/bin/byobu-tmux new-session"),
            process(6, "dtach -n /tmp/work bash"),
            process(7, "abduco -n work bash"),
            process(8, "mosh-server new -s -c 256"),
        ];
        let outside = [false; SESSION_GUARDS.len()];

        assert_eq!(
            session_warnings(
                "zellij\ntmux\nopenssh\nscreen\nbyobu\ndtach\nabduco\nmosh\n",
                &outside,
                &processes
            ),
            [
                ZELLIJ_SERVER_WARNING,
                TMUX_WARNING,
                SSH_WARNING,
                SCREEN_WARNING,
                BYOBU_WARNING,
                DTACH_WARNING,
                ABDUCO_WARNING,
                MOSH_WARNING,
            ]
        );
    }

    #[test]
    fn session_warnings_only_apply_to_outdated_packages() {
        let processes = [process(1, "zellij --server /tmp/zellij-1000")];

        let inside = [true; SESSION_GUARDS.len()];

        assert!(session_warnings("node\n", &inside, &processes).is_empty());
        assert!(session_warnings("zellij-utils\n", &inside, &processes).is_empty());
        assert!(session_warnings("", &inside, &processes).is_empty());
    }

    #[test]
    fn the_session_guard_table_has_an_entry_per_tool() {
        assert_eq!(SESSION_GUARDS.len(), 8);
        for guard in SESSION_GUARDS {
            assert!(!guard.packages.is_empty());
            assert!(guard.environment.iter().all(|name| !name.is_empty()));
        }
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
    fn screen_byobu_dtach_abduco_and_mosh_sessions_are_detected() {
        assert!(is_screen_process("/usr/bin/SCREEN -S work"));
        assert!(is_screen_process("screen -ls"));
        assert!(!is_screen_process("screenplay"));

        assert!(is_byobu_process("/usr/bin/byobu"));
        assert!(is_byobu_process("/usr/bin/byobu-tmux new-session"));
        assert!(is_byobu_process("tmux -L byobu new-session"));
        assert!(!is_byobu_process("/opt/homebrew/bin/tmux new-session"));

        assert!(is_dtach_process("dtach -n /tmp/work bash"));
        assert!(!is_dtach_process("dtach-helper"));

        assert!(is_abduco_process("abduco -n work bash"));
        assert!(!is_abduco_process("abduco-helper"));

        assert!(is_mosh_process("mosh-server new -s -c 256"));
        assert!(!is_mosh_process("mosh user@example.com"));
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
    fn windows_process_table_keeps_parent_pid_and_the_full_command_line() {
        let separator = '\u{1f}';
        let table = format!(
            "100{separator}1{separator}C:\\Users\\me\\AppData\\Local\\mise\\installs\\node\\20.0.0\\node.exe C:\\path with spaces\\server.js\r\n\
             101{separator}100{separator}\"C:\\Program Files\\LM Studio\\LM Studio.exe\" --serve\r\n\
             102{separator}1{separator}\r\n"
        );

        let processes = parse_windows_process_table(&table);

        assert_eq!(processes.len(), 2);
        assert_eq!(processes[0].pid, 100);
        assert_eq!(processes[0].ppid, 1);
        assert_eq!(
            processes[0].command,
            r"C:\Users\me\AppData\Local\mise\installs\node\20.0.0\node.exe C:\path with spaces\server.js"
        );
        assert_eq!(processes[1].ppid, 100);
        assert_eq!(
            processes[1].command,
            r#""C:\Program Files\LM Studio\LM Studio.exe" --serve"#
        );
    }

    #[test]
    fn windows_install_paths_are_matched_with_either_separator() {
        let separator = '\u{1f}';
        let prunable = r"C:\Users\me\AppData\Local\mise\installs\node\20.0.0";
        let table = format!(
            "100{separator}1{separator}C:\\Users\\me\\AppData\\Local\\mise\\installs\\node\\22.1.0\\node.exe C:\\current\\server.js\r\n\
             101{separator}1{separator}C:\\Users\\me\\AppData\\Local\\mise\\installs\\node\\20.0.0\\node.exe C:\\old\\server.js\r\n\
             102{separator}1{separator}C:\\Program Files\\nodejs\\node.exe C:\\system\\server.js\r\n\
             103{separator}1{separator}C:/Users/me/AppData/Local/mise/installs/node/20.0.0/node.exe C:/old/forward.js\r\n\
             104{separator}1{separator}C:\\Users\\me\\AppData\\Local\\mise\\installs\\node\\24.0.0\\node.exe C:\\newer\\server.js\r\n"
        );
        let processes = parse_windows_process_table(&table);

        let stale = stale_tool_entries(&processes, &[Path::new(prunable)]);

        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].pid, 101);
        assert_eq!(stale[1].pid, 103);
    }

    struct FakeProcessProbe {
        processes: Vec<ProcessEntry>,
    }

    impl ProcessProbe for FakeProcessProbe {
        fn processes(&self) -> Vec<ProcessEntry> {
            self.processes.clone()
        }
    }

    #[test]
    fn parent_names_are_read_from_the_injected_process_table() {
        let probe = FakeProcessProbe {
            processes: vec![
                ProcessEntry {
                    pid: 1,
                    ppid: 0,
                    command: r#""C:\Program Files\LM Studio\LM Studio.exe" --serve"#.to_owned(),
                },
                ProcessEntry {
                    pid: 100,
                    ppid: 1,
                    command: r"C:\Users\me\AppData\Local\mise\installs\node\20.0.0\node.exe C:\old\server.js".to_owned(),
                },
            ],
        };
        let processes = probe.processes();

        assert_eq!(probe.parent_name(1, &processes), "LM Studio");
        assert_eq!(probe.parent_name(999, &processes), "");
    }

    #[test]
    fn program_names_handle_windows_paths_and_quotes() {
        assert_eq!(
            program_name(r#""C:\Program Files\nodejs\node.exe" script.js"#),
            "node"
        );
        assert_eq!(program_name(r"C:\tools\zellij.exe --server"), "zellij");
        assert_eq!(program_name(r"D:\Docker\docker.exe build ."), "docker");
    }

    #[test]
    fn only_processes_from_versions_mise_will_prune_are_stale() {
        let table = format!(
            "\
  100 1 {ACTIVE_NODE}/bin/node /current.js
  101 1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node /old.js
  102 1 /usr/bin/node /system.js
  103 1 /home/user/.local/share/mise/installs/node/20.0.0/bin/node-foo /old.js
  104 1 /home/user/.local/share/mise/installs/node/24.0.0/bin/node /newer.js
  200 1 {ACTIVE_PYTHON}/bin/python3 /current.py
  201 1 /home/user/.local/share/mise/installs/python/3.10.0/bin/python3 /old.py
 "
        );
        let processes = parse_process_table(&table);

        let prunable_node = [Path::new(
            "/home/user/.local/share/mise/installs/node/20.0.0",
        )];
        let stale = stale_tool_entries(&processes, &prunable_node);

        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].pid, 101);
        assert_eq!(stale[1].pid, 103);

        let prunable_python = [Path::new(
            "/home/user/.local/share/mise/installs/python/3.10.0",
        )];
        let stale = stale_tool_entries(&processes, &prunable_python);

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].pid, 201);
    }

    #[test]
    fn prunable_versions_are_read_from_the_install_listing() {
        let json = r#"{
            "node": [
                {"version": "20.0.0", "install_path": "/home/user/.local/share/mise/installs/node/20.0.0"},
                {"version": "18.0.0", "install_path": "/home/user/.local/share/mise/installs/node/18.0.0"}
            ],
            "ruby": [
                {"version": "3.3.0", "install_path": "/home/user/.local/share/mise/installs/ruby/3.3.0"}
            ],
            "python": [
                {"version": "3.10.0"},
                {"version": "", "install_path": ""}
            ]
        }"#;

        let prunable = parse_prunable_tool_versions(json);

        assert_eq!(
            prunable,
            [
                PrunableTool {
                    tool: "node".to_owned(),
                    path: PathBuf::from("/home/user/.local/share/mise/installs/node/20.0.0"),
                },
                PrunableTool {
                    tool: "node".to_owned(),
                    path: PathBuf::from("/home/user/.local/share/mise/installs/node/18.0.0"),
                },
            ]
        );
    }

    #[test]
    fn unreadable_prunable_listings_are_ignored() {
        assert!(parse_prunable_tool_versions("").is_empty());
        assert!(parse_prunable_tool_versions("not json").is_empty());
        assert!(parse_prunable_tool_versions("[]").is_empty());
        assert!(parse_prunable_tool_versions(r#"{"node": "20.0.0"}"#).is_empty());
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
