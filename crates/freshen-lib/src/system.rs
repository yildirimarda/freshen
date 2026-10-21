//! Detection of the host environment a run is executing on.
//!
//! Steps need more than the compiled-for [`Platform`]: the Linux distribution
//! decides which package manager exists, the architecture decides which
//! downloads are valid, the privilege level decides whether a step can use
//! `sudo`, and the terminal decides whether freshen may prompt or must pick
//! safe defaults. The terminal application is recorded so a later step can
//! avoid quitting the terminal that is running freshen.
//!
//! Detection goes through [`SystemProbe`], so tests describe a host with fixed
//! facts instead of reading the machine they run on. [`SystemInfo::detect`]
//! uses the real host.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::IsTerminal;
use std::path::Path;
#[cfg(any(unix, windows))]
use std::process::{Command, Stdio};

use crate::platform::Platform;

/// Whether freshen is running with elevated privileges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Privilege {
    /// A normal user account.
    StandardUser,
    /// Root on Unix, or an administrator on Windows.
    Elevated,
    /// The privilege level could not be determined.
    Unknown,
}

impl Privilege {
    /// Returns `true` when the process has elevated privileges.
    #[must_use]
    pub fn is_elevated(self) -> bool {
        matches!(self, Self::Elevated)
    }

    /// Lowercase name used in configuration, CLI arguments and logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StandardUser => "standard",
            Self::Elevated => "elevated",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Privilege {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The CPU architecture freshen is running on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Arch {
    /// 32-bit x86.
    X86,
    /// 64-bit x86.
    X86_64,
    /// 32-bit ARM.
    Arm,
    /// 64-bit ARM.
    Aarch64,
    /// 64-bit RISC-V.
    Riscv64,
    /// 32-bit PowerPC.
    Powerpc,
    /// 64-bit PowerPC, including little-endian variants.
    Powerpc64,
    /// 64-bit IBM Z.
    S390x,
    /// An architecture freshen does not name explicitly.
    Other(String),
}

impl Arch {
    /// Maps a Rust target architecture name to an [`Arch`].
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match name {
            "x86" | "i386" | "i586" | "i686" => Self::X86,
            "x86_64" | "amd64" => Self::X86_64,
            "arm" | "armv6l" | "armv7l" => Self::Arm,
            "aarch64" | "arm64" => Self::Aarch64,
            "riscv64" => Self::Riscv64,
            "powerpc" | "ppc" => Self::Powerpc,
            "powerpc64" | "ppc64" | "ppc64le" => Self::Powerpc64,
            "s390x" => Self::S390x,
            other => Self::Other(other.to_string()),
        }
    }

    /// The architecture this binary was compiled for.
    #[must_use]
    pub fn current() -> Self {
        Self::parse(std::env::consts::ARCH)
    }

    /// Lowercase name used in configuration, CLI arguments and logs.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
            Self::Arm => "arm",
            Self::Aarch64 => "aarch64",
            Self::Riscv64 => "riscv64",
            Self::Powerpc => "powerpc",
            Self::Powerpc64 => "powerpc64",
            Self::S390x => "s390x",
            Self::Other(name) => name,
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A Linux distribution read from `/etc/os-release`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distro {
    id: String,
    name: Option<String>,
    version: Option<String>,
    id_like: Vec<String>,
}

impl Distro {
    /// Parses the contents of an `os-release` file.
    ///
    /// Returns `None` when the file has no usable `ID`, because the id is what
    /// package-manager selection keys on. Unknown keys, comments, blank lines
    /// and malformed lines are ignored.
    #[must_use]
    pub fn parse(contents: &str) -> Option<Self> {
        let fields = parse_os_release(contents);
        let id = fields.get("ID")?.trim().to_string();
        if id.is_empty() {
            return None;
        }

        let name = non_empty(fields.get("NAME"));
        let version = non_empty(fields.get("VERSION_ID"));
        let id_like = fields
            .get("ID_LIKE")
            .map(|value| value.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();

        Some(Self {
            id,
            name,
            version,
            id_like,
        })
    }

    /// The lowercase distribution id, for example `ubuntu` or `fedora`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The human-readable distribution name, when the file provides one.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The distribution version, when the file provides one.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Distributions this one is derived from, for example `debian` for Ubuntu.
    #[must_use]
    pub fn id_like(&self) -> &[String] {
        &self.id_like
    }

    /// The best label for logs: the pretty name when available, else the id.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

impl fmt::Display for Distro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

fn parse_os_release(contents: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        fields.insert(key.to_string(), parse_os_release_value(value.trim()));
    }
    fields
}

fn parse_os_release_value(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return unescape(inner);
    }
    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return inner.to_string();
    }
    value.to_string()
}

fn unescape(value: &str) -> String {
    let mut unescaped = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => unescaped.push('\n'),
            Some('t') => unescaped.push('\t'),
            Some(escaped) => unescaped.push(escaped),
            None => unescaped.push('\\'),
        }
    }
    unescaped
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The terminal application running freshen.
///
/// `bundle_id` is the macOS bundle identifier when it is known, which is what
/// the app quit/reopen logic compares against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalApp {
    name: String,
    bundle_id: Option<String>,
}

impl TerminalApp {
    /// Creates a terminal application description.
    #[must_use]
    pub fn new(name: impl Into<String>, bundle_id: Option<String>) -> Self {
        Self {
            name: name.into(),
            bundle_id,
        }
    }

    /// The application name, for example `iTerm2` or `Windows Terminal`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The macOS bundle identifier, when the application has one.
    #[must_use]
    pub fn bundle_id(&self) -> Option<&str> {
        self.bundle_id.as_deref()
    }
}

impl fmt::Display for TerminalApp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// A terminal emulator freshen recognises.
///
/// The same table drives every detection source and the README list, so
/// adding a terminal is one row.
struct TerminalKind {
    /// Name shown to the user and in the README.
    name: &'static str,
    /// macOS bundle identifiers that identify this terminal.
    bundle_ids: &'static [&'static str],
    /// `TERM_PROGRAM` values that identify this terminal.
    term_programs: &'static [&'static str],
    /// Environment variables set inside this terminal.
    markers: &'static [&'static str],
    /// Executable names from the process table.
    process_names: &'static [&'static str],
}

/// Terminal emulators freshen can recognise, in documentation order.
const TERMINALS: &[TerminalKind] = &[
    TerminalKind {
        name: "Apple Terminal",
        bundle_ids: &["com.apple.Terminal"],
        term_programs: &["Apple_Terminal"],
        markers: &[],
        process_names: &[],
    },
    TerminalKind {
        name: "iTerm2",
        bundle_ids: &["com.googlecode.iterm2"],
        term_programs: &["iTerm.app"],
        markers: &[],
        process_names: &[],
    },
    TerminalKind {
        name: "Ghostty",
        bundle_ids: &["com.mitchellh.ghostty"],
        term_programs: &["ghostty"],
        markers: &[],
        process_names: &["ghostty"],
    },
    TerminalKind {
        name: "kitty",
        bundle_ids: &["net.kovidgoyal.kitty"],
        term_programs: &["kitty"],
        markers: &["KITTY_WINDOW_ID"],
        process_names: &["kitty"],
    },
    TerminalKind {
        name: "WezTerm",
        bundle_ids: &["com.github.wez.wezterm"],
        term_programs: &["WezTerm"],
        markers: &["WEZTERM_PANE"],
        process_names: &["wezterm", "wezterm-gui"],
    },
    TerminalKind {
        name: "Alacritty",
        bundle_ids: &["org.alacritty"],
        term_programs: &["Alacritty"],
        markers: &["ALACRITTY_WINDOW_ID"],
        process_names: &["alacritty"],
    },
    TerminalKind {
        name: "Hyper",
        bundle_ids: &["co.zeit.hyper"],
        term_programs: &["Hyper"],
        markers: &[],
        process_names: &["hyper"],
    },
    TerminalKind {
        name: "Warp",
        bundle_ids: &["dev.warp.Warp-Stable"],
        term_programs: &["WarpTerminal"],
        markers: &[],
        process_names: &["warp", "warp-terminal"],
    },
    TerminalKind {
        name: "Tabby",
        bundle_ids: &[],
        term_programs: &["Tabby"],
        markers: &[],
        process_names: &["tabby"],
    },
    TerminalKind {
        name: "Rio",
        bundle_ids: &["com.raphaelamorim.rio"],
        term_programs: &["rio"],
        markers: &[],
        process_names: &["rio"],
    },
    TerminalKind {
        name: "Termius",
        bundle_ids: &["com.termius-dmg.mac"],
        term_programs: &["Termius"],
        markers: &[],
        process_names: &["termius"],
    },
    TerminalKind {
        name: "Visual Studio Code",
        bundle_ids: &["com.microsoft.VSCode"],
        term_programs: &["vscode"],
        markers: &["VSCODE_IPC_HOOK_CLI"],
        process_names: &["code", "code-insiders"],
    },
    TerminalKind {
        name: "Zed",
        bundle_ids: &["dev.zed.Zed"],
        term_programs: &["zed"],
        markers: &["ZED_TERM"],
        process_names: &["zed"],
    },
    TerminalKind {
        name: "Zed Preview",
        bundle_ids: &["dev.zed.Zed-Preview"],
        term_programs: &[],
        markers: &[],
        process_names: &[],
    },
    TerminalKind {
        name: "Cursor",
        bundle_ids: &["com.todesktop.230313mzl4w4u92"],
        term_programs: &["cursor"],
        markers: &["CURSOR_TRACE_ID"],
        process_names: &["cursor"],
    },
    TerminalKind {
        name: "Windsurf",
        bundle_ids: &["com.exafunction.windsurf"],
        term_programs: &["windsurf"],
        markers: &["WINDSURF_SESSION"],
        process_names: &["windsurf"],
    },
    TerminalKind {
        name: "JetBrains",
        bundle_ids: &[],
        term_programs: &["JetBrains-JediTerm"],
        markers: &["TERMINAL_EMULATOR"],
        process_names: &[],
    },
    TerminalKind {
        name: "IntelliJ IDEA",
        bundle_ids: &["com.jetbrains.intellij"],
        term_programs: &[],
        markers: &[],
        process_names: &["idea"],
    },
    TerminalKind {
        name: "IntelliJ IDEA CE",
        bundle_ids: &["com.jetbrains.intellij.ce"],
        term_programs: &[],
        markers: &[],
        process_names: &[],
    },
    TerminalKind {
        name: "PyCharm",
        bundle_ids: &["com.jetbrains.pycharm"],
        term_programs: &[],
        markers: &[],
        process_names: &["pycharm"],
    },
    TerminalKind {
        name: "PyCharm CE",
        bundle_ids: &["com.jetbrains.pycharm.ce"],
        term_programs: &[],
        markers: &[],
        process_names: &[],
    },
    TerminalKind {
        name: "WebStorm",
        bundle_ids: &["com.jetbrains.WebStorm"],
        term_programs: &[],
        markers: &[],
        process_names: &["webstorm"],
    },
    TerminalKind {
        name: "CLion",
        bundle_ids: &["com.jetbrains.CLion"],
        term_programs: &[],
        markers: &[],
        process_names: &["clion"],
    },
    TerminalKind {
        name: "GoLand",
        bundle_ids: &["com.jetbrains.goland"],
        term_programs: &[],
        markers: &[],
        process_names: &["goland"],
    },
    TerminalKind {
        name: "RubyMine",
        bundle_ids: &["com.jetbrains.rubymine"],
        term_programs: &[],
        markers: &[],
        process_names: &["rubymine"],
    },
    TerminalKind {
        name: "PhpStorm",
        bundle_ids: &["com.jetbrains.PhpStorm"],
        term_programs: &[],
        markers: &[],
        process_names: &["phpstorm"],
    },
    TerminalKind {
        name: "DataGrip",
        bundle_ids: &["com.jetbrains.datagrip"],
        term_programs: &[],
        markers: &[],
        process_names: &["datagrip"],
    },
    TerminalKind {
        name: "Rider",
        bundle_ids: &["com.jetbrains.rider"],
        term_programs: &[],
        markers: &[],
        process_names: &["rider"],
    },
    TerminalKind {
        name: "AppCode",
        bundle_ids: &["com.jetbrains.AppCode"],
        term_programs: &[],
        markers: &[],
        process_names: &[],
    },
    TerminalKind {
        name: "RustRover",
        bundle_ids: &["com.jetbrains.rustrover"],
        term_programs: &[],
        markers: &[],
        process_names: &["rustrover"],
    },
    TerminalKind {
        name: "Fleet",
        bundle_ids: &["com.jetbrains.fleet"],
        term_programs: &[],
        markers: &[],
        process_names: &["fleet"],
    },
    TerminalKind {
        name: "Android Studio",
        bundle_ids: &["com.google.android.studio"],
        term_programs: &[],
        markers: &[],
        process_names: &["studio.sh", "studio"],
    },
    TerminalKind {
        name: "GNOME Terminal",
        bundle_ids: &["org.gnome.Terminal"],
        term_programs: &[],
        markers: &["GNOME_TERMINAL_SCREEN"],
        process_names: &["gnome-terminal", "gnome-terminal-server"],
    },
    TerminalKind {
        name: "Konsole",
        bundle_ids: &["org.kde.konsole"],
        term_programs: &[],
        markers: &["KONSOLE_VERSION"],
        process_names: &["konsole"],
    },
    TerminalKind {
        name: "Windows Terminal",
        bundle_ids: &[],
        term_programs: &[],
        markers: &["WT_SESSION"],
        process_names: &["windowsterminal", "wt"],
    },
    TerminalKind {
        name: "Windows Console",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["conhost", "openconsole"],
    },
    TerminalKind {
        name: "xterm",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["xterm"],
    },
    TerminalKind {
        name: "Xfce Terminal",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["xfce4-terminal"],
    },
    TerminalKind {
        name: "LXTerminal",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["lxterminal"],
    },
    TerminalKind {
        name: "MATE Terminal",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["mate-terminal"],
    },
    TerminalKind {
        name: "Terminator",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["terminator"],
    },
    TerminalKind {
        name: "Tilix",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["tilix"],
    },
    TerminalKind {
        name: "urxvt",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["urxvt"],
    },
    TerminalKind {
        name: "foot",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["foot"],
    },
    TerminalKind {
        name: "GNOME Console",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["kgx"],
    },
    TerminalKind {
        name: "st",
        bundle_ids: &[],
        term_programs: &[],
        markers: &[],
        process_names: &["st"],
    },
];

/// Terminal emulators freshen can recognise, in documentation order.
#[must_use]
pub fn supported_terminals() -> Vec<&'static str> {
    TERMINALS.iter().map(|terminal| terminal.name).collect()
}

fn terminal_for_bundle_id(bundle_id: &str) -> Option<&'static TerminalKind> {
    TERMINALS
        .iter()
        .find(|terminal| terminal.bundle_ids.contains(&bundle_id))
}

fn terminal_for_program(program: &str) -> Option<&'static TerminalKind> {
    TERMINALS
        .iter()
        .find(|terminal| terminal.term_programs.contains(&program))
}

fn terminal_name_for_program(program: &str) -> &str {
    terminal_for_program(program).map_or(program, |terminal| terminal.name)
}

fn bundle_id_for_program(program: &str) -> Option<&'static str> {
    terminal_for_program(program).and_then(|terminal| terminal.bundle_ids.first().copied())
}

pub(crate) fn detect_terminal_app(get: impl Fn(&str) -> Option<String>) -> Option<TerminalApp> {
    let bundle_id = get("__CFBundleIdentifier").filter(|value| !value.is_empty());
    let term_program = get("TERM_PROGRAM").filter(|value| !value.is_empty());

    if let Some(bundle_id) = bundle_id {
        let name = terminal_for_bundle_id(&bundle_id)
            .map(|terminal| terminal.name.to_string())
            .or_else(|| {
                term_program
                    .as_deref()
                    .map(|program| terminal_name_for_program(program).to_string())
            })
            .unwrap_or_else(|| bundle_id.clone());
        return Some(TerminalApp::new(name, Some(bundle_id)));
    }

    if let Some(program) = term_program {
        return Some(TerminalApp::new(
            terminal_name_for_program(&program),
            bundle_id_for_program(&program).map(str::to_string),
        ));
    }

    for terminal in TERMINALS {
        if terminal
            .markers
            .iter()
            .any(|variable| get(variable).is_some_and(|value| !value.is_empty()))
        {
            return Some(TerminalApp::new(
                terminal.name,
                terminal.bundle_ids.first().map(|id| (*id).to_string()),
            ));
        }
    }

    None
}

/// Maps an executable name from the process table to a terminal application.
///
/// The name may be a bare executable (`konsole`), a full path
/// (`/usr/bin/konsole`) or a Windows executable (`WindowsTerminal.exe`).
fn terminal_app_for_process(name: &str) -> Option<TerminalApp> {
    let file_name = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(name);
    let stem = file_name
        .strip_suffix(".exe")
        .or_else(|| file_name.strip_suffix(".EXE"))
        .unwrap_or(file_name);
    let key = stem.to_ascii_lowercase();

    let terminal = TERMINALS
        .iter()
        .find(|terminal| terminal.process_names.contains(&key.as_str()))?;

    Some(TerminalApp::new(
        terminal.name,
        terminal.bundle_ids.first().map(|id| (*id).to_string()),
    ))
}

/// Walks from `current_pid` towards the root of the process table and returns
/// the first ancestor that is a known terminal application.
///
/// Intermediate processes (shells, `tmux`, `sudo`) are skipped, and a missing
/// parent or a cycle ends the walk instead of looping forever.
fn detect_terminal_app_from_processes(
    current_pid: u32,
    processes: &[ProcessInfo],
) -> Option<TerminalApp> {
    let by_pid: HashMap<u32, &ProcessInfo> = processes.iter().map(|p| (p.pid, p)).collect();
    let mut pid = current_pid;
    let mut seen = HashSet::new();

    while seen.insert(pid) {
        let process = by_pid.get(&pid)?;
        if let Some(app) = terminal_app_for_process(&process.name) {
            return Some(app);
        }
        if process.ppid == 0 || process.ppid == pid {
            break;
        }
        pid = process.ppid;
    }

    None
}

/// Detects the terminal application from environment markers, then falls back
/// to the parent process chain on Linux and Windows.
fn detect_terminal_app_with_processes(
    platform: Option<Platform>,
    get: impl Fn(&str) -> Option<String>,
    current_pid: u32,
    processes: &[ProcessInfo],
) -> Option<TerminalApp> {
    if let Some(app) = detect_terminal_app(get) {
        return Some(app);
    }

    if matches!(platform, Some(Platform::Linux | Platform::Windows)) {
        return detect_terminal_app_from_processes(current_pid, processes);
    }

    None
}

/// The terminal attached to this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    app: Option<TerminalApp>,
}

impl Terminal {
    /// Returns `true` when both input and output are attached to a terminal.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        self.stdin_is_terminal && self.stdout_is_terminal
    }

    /// Whether standard input is attached to a terminal.
    #[must_use]
    pub fn stdin_is_terminal(&self) -> bool {
        self.stdin_is_terminal
    }

    /// Whether standard output is attached to a terminal.
    #[must_use]
    pub fn stdout_is_terminal(&self) -> bool {
        self.stdout_is_terminal
    }

    /// The terminal application running freshen, when it can be identified.
    #[must_use]
    pub fn app(&self) -> Option<&TerminalApp> {
        self.app.as_ref()
    }
}

/// A virtualized environment the operating system may be running in.
///
/// WSL and containers are not mutually exclusive: a container can run inside
/// a WSL distribution. Steps that change the host system can use this to adapt
/// to WSL or skip work inside a container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    wsl: bool,
    wsl_distro: Option<String>,
    container: bool,
}

impl Environment {
    /// Returns `true` when the host is neither WSL nor a container.
    #[must_use]
    pub fn is_native(&self) -> bool {
        !self.wsl && !self.container
    }

    /// Returns `true` when running inside Windows Subsystem for Linux.
    #[must_use]
    pub fn is_wsl(&self) -> bool {
        self.wsl
    }

    /// The WSL distribution name from `WSL_DISTRO_NAME`, when it was set.
    #[must_use]
    pub fn wsl_distro(&self) -> Option<&str> {
        self.wsl_distro.as_deref()
    }

    /// Returns `true` when running inside a container such as Docker or Podman.
    #[must_use]
    pub fn is_container(&self) -> bool {
        self.container
    }
}

/// The operating system freshen is running on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsInfo {
    name: String,
    platform: Option<Platform>,
    distro: Option<Distro>,
    environment: Environment,
}

impl OsInfo {
    /// The operating system name reported by Rust, for example `linux`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The supported platform, or `None` on an unsupported operating system.
    #[must_use]
    pub fn platform(&self) -> Option<Platform> {
        self.platform
    }

    /// The Linux distribution, when one was detected.
    #[must_use]
    pub fn distro(&self) -> Option<&Distro> {
        self.distro.as_ref()
    }

    /// The virtualized environment the operating system is running in.
    #[must_use]
    pub fn environment(&self) -> &Environment {
        &self.environment
    }
}

/// Facts about the machine and terminal freshen is running on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    os: OsInfo,
    arch: Arch,
    privilege: Privilege,
    terminal: Terminal,
}

impl SystemInfo {
    /// Detects the host environment.
    #[must_use]
    pub fn detect() -> Self {
        Self::detect_with(&HostProbe)
    }

    /// Builds the environment from the facts provided by `probe`.
    #[must_use]
    pub fn detect_with(probe: &dyn SystemProbe) -> Self {
        let os_name = probe.os().to_string();
        let platform = Platform::from_name(&os_name);
        let distro = probe.os_release().as_deref().and_then(Distro::parse);
        let environment = detect_environment(probe);
        let terminal = Terminal {
            stdin_is_terminal: probe.stdin_is_terminal(),
            stdout_is_terminal: probe.stdout_is_terminal(),
            app: detect_terminal_app_with_processes(
                platform,
                |key| probe.var(key),
                probe.current_pid(),
                &probe.processes(),
            ),
        };

        Self {
            os: OsInfo {
                name: os_name,
                platform,
                distro,
                environment,
            },
            arch: Arch::parse(probe.arch()),
            privilege: probe.privilege(),
            terminal,
        }
    }

    /// Operating system information.
    #[must_use]
    pub fn os(&self) -> &OsInfo {
        &self.os
    }

    /// The supported platform, or `None` on an unsupported operating system.
    #[must_use]
    pub fn platform(&self) -> Option<Platform> {
        self.os.platform()
    }

    /// The virtualized environment the operating system is running in.
    #[must_use]
    pub fn environment(&self) -> &Environment {
        self.os.environment()
    }

    /// The CPU architecture.
    #[must_use]
    pub fn arch(&self) -> &Arch {
        &self.arch
    }

    /// The privilege level of the current process.
    #[must_use]
    pub fn privilege(&self) -> Privilege {
        self.privilege
    }

    /// The terminal attached to the process.
    #[must_use]
    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// Returns `true` when freshen may prompt the user.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        self.terminal.is_interactive()
    }
}

/// One entry in a host process table.
///
/// `name` is the executable name or path reported by the operating system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Process id.
    pub pid: u32,
    /// Parent process id.
    pub ppid: u32,
    /// Executable name or path.
    pub name: String,
}

/// Raw facts about the host, used by [`SystemInfo::detect_with`].
///
/// The real implementation is [`HostProbe`]. Tests and embedders can provide
/// fixed facts without reading the machine.
pub trait SystemProbe {
    /// Operating system name, matching [`std::env::consts::OS`].
    fn os(&self) -> &str;

    /// CPU architecture name, matching [`std::env::consts::ARCH`].
    fn arch(&self) -> &str;

    /// Contents of the Linux `os-release` file, when available.
    fn os_release(&self) -> Option<String>;

    /// Contents of a file, when it exists and is readable as UTF-8.
    fn read_file(&self, path: &str) -> Option<String>;

    /// Whether a path exists.
    fn path_exists(&self, path: &str) -> bool;

    /// The privilege level of the current process.
    fn privilege(&self) -> Privilege;

    /// Whether standard input is attached to a terminal.
    fn stdin_is_terminal(&self) -> bool;

    /// Whether standard output is attached to a terminal.
    fn stdout_is_terminal(&self) -> bool;

    /// Reads an environment variable, returning `None` when it is unset or not
    /// valid Unicode.
    fn var(&self, key: &str) -> Option<String>;

    /// The process id of the running freshen process.
    fn current_pid(&self) -> u32 {
        std::process::id()
    }

    /// A snapshot of the host process table.
    fn processes(&self) -> Vec<ProcessInfo> {
        Vec::new()
    }
}

/// The real host freshen is running on.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostProbe;

impl SystemProbe for HostProbe {
    fn os(&self) -> &str {
        std::env::consts::OS
    }

    fn arch(&self) -> &str {
        std::env::consts::ARCH
    }

    fn os_release(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/etc/os-release").ok()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn path_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn privilege(&self) -> Privilege {
        detect_privilege()
    }

    fn stdin_is_terminal(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn stdout_is_terminal(&self) -> bool {
        std::io::stdout().is_terminal()
    }

    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn current_pid(&self) -> u32 {
        std::process::id()
    }

    fn processes(&self) -> Vec<ProcessInfo> {
        #[cfg(target_os = "linux")]
        {
            linux_processes()
        }
        #[cfg(all(unix, not(target_os = "linux")))]
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
}

/// Reads the process table from `/proc`, which is available on every Linux
/// system without spawning an external command.
#[cfg(target_os = "linux")]
fn linux_processes() -> Vec<ProcessInfo> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let pid = entry.ok()?.file_name().to_str()?.parse().ok()?;
            linux_process_info(pid)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_process_info(pid: u32) -> Option<ProcessInfo> {
    let directory = format!("/proc/{pid}");
    let stat = std::fs::read_to_string(format!("{directory}/stat")).ok()?;
    let (ppid, comm) = parse_proc_stat(&stat)?;
    let name = std::fs::read_to_string(format!("{directory}/cmdline"))
        .ok()
        .and_then(|cmdline| cmdline.split('\0').next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .unwrap_or(comm);

    Some(ProcessInfo { pid, ppid, name })
}

/// Parses the parent id and command name from a `/proc/<pid>/stat` line.
///
/// The command is wrapped in parentheses and may itself contain spaces or
/// parentheses, so the last `)` separates it from the numeric fields.
#[cfg(target_os = "linux")]
fn parse_proc_stat(stat: &str) -> Option<(u32, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    let mut fields = stat.get(close + 1..)?.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;

    Some((ppid, name))
}

/// Reads the process table with `ps` on Unix systems that do not expose
/// `/proc` (macOS and the BSDs).
#[cfg(all(unix, not(target_os = "linux")))]
fn ps_processes() -> Vec<ProcessInfo> {
    let Ok(output) = Command::new("ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_ps_processes(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn parse_ps_processes(table: &str) -> Vec<ProcessInfo> {
    table
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start();
            let pid_end = rest.find(char::is_whitespace)?;
            let pid = rest.get(..pid_end)?.parse().ok()?;
            let rest = rest.get(pid_end..)?.trim_start();
            let ppid_end = rest.find(char::is_whitespace)?;
            let ppid = rest.get(..ppid_end)?.parse().ok()?;
            let name = rest.get(ppid_end..)?.trim_start().to_string();
            if name.is_empty() {
                return None;
            }
            Some(ProcessInfo { pid, ppid, name })
        })
        .collect()
}

/// Reads the process table through CIM on Windows.
#[cfg(windows)]
fn windows_processes() -> Vec<ProcessInfo> {
    let script = "Get-CimInstance Win32_Process | ForEach-Object { \
                  \"$($_.ProcessId) $($_.ParentProcessId) $($_.Name)\" }";
    let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_windows_processes(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(windows)]
fn parse_windows_processes(table: &str) -> Vec<ProcessInfo> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let name = fields.next()?.to_string();
            Some(ProcessInfo { pid, ppid, name })
        })
        .collect()
}

/// Markers found in `/proc/1/cgroup` inside common container runtimes.
const CONTAINER_CGROUP_MARKERS: [&str; 5] = ["docker", "containerd", "kubepods", "lxc", "podman"];

/// Detects WSL and containers from the facts provided by `probe`.
///
/// WSL is identified by `WSL_DISTRO_NAME`, `WSL_INTEROP` or a `/proc/version`
/// containing `microsoft` or `wsl`. Containers are identified by the Docker and
/// Podman marker files, the `container` environment variable, or a container
/// runtime in `/proc/1/cgroup`. Only Linux can be WSL or a Linux container.
fn detect_environment(probe: &dyn SystemProbe) -> Environment {
    if Platform::from_name(probe.os()) != Some(Platform::Linux) {
        return Environment::default();
    }

    let proc_version = probe
        .read_file("/proc/version")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let wsl_distro = probe
        .var("WSL_DISTRO_NAME")
        .filter(|value| !value.is_empty());
    let wsl = wsl_distro.is_some()
        || probe
            .var("WSL_INTEROP")
            .is_some_and(|value| !value.is_empty())
        || proc_version.contains("microsoft")
        || proc_version.contains("wsl");

    let cgroup = probe
        .read_file("/proc/1/cgroup")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let container = probe.path_exists("/.dockerenv")
        || probe.path_exists("/run/.containerenv")
        || probe
            .var("container")
            .is_some_and(|value| !value.is_empty())
        || CONTAINER_CGROUP_MARKERS
            .iter()
            .any(|marker| cgroup.contains(marker));

    Environment {
        wsl,
        wsl_distro,
        container,
    }
}

/// Detects elevation without unsafe code: `id -u` on Unix and `net session`
/// on Windows. Returns [`Privilege::Unknown`] when the probe cannot run.
fn detect_privilege() -> Privilege {
    #[cfg(unix)]
    {
        let output = Command::new("id")
            .arg("-u")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let uid = String::from_utf8_lossy(&output.stdout);
                if uid.trim() == "0" {
                    Privilege::Elevated
                } else {
                    Privilege::StandardUser
                }
            }
            _ => Privilege::Unknown,
        }
    }
    #[cfg(windows)]
    {
        let status = Command::new("net")
            .arg("session")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => Privilege::Elevated,
            Ok(_) => Privilege::StandardUser,
            Err(_) => Privilege::Unknown,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        Privilege::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_terminal_app, terminal_app_for_process, Arch, Distro, Environment, Privilege,
        ProcessInfo, SystemInfo, SystemProbe, Terminal, TerminalApp,
    };
    use crate::platform::Platform;

    struct FakeProbe {
        os: &'static str,
        arch: &'static str,
        os_release: Option<String>,
        privilege: Privilege,
        stdin_is_terminal: bool,
        stdout_is_terminal: bool,
        vars: Vec<(&'static str, &'static str)>,
        files: Vec<(String, String)>,
        current_pid: u32,
        processes: Vec<ProcessInfo>,
    }

    impl FakeProbe {
        fn new(os: &'static str, arch: &'static str) -> Self {
            Self {
                os,
                arch,
                os_release: None,
                privilege: Privilege::StandardUser,
                stdin_is_terminal: true,
                stdout_is_terminal: true,
                vars: Vec::new(),
                files: Vec::new(),
                current_pid: 1,
                processes: Vec::new(),
            }
        }

        fn with_os_release(mut self, contents: &str) -> Self {
            self.os_release = Some(contents.to_string());
            self
        }

        fn with_var(mut self, key: &'static str, value: &'static str) -> Self {
            self.vars.push((key, value));
            self
        }

        fn with_file(mut self, path: &str, contents: &str) -> Self {
            self.files.push((path.to_string(), contents.to_string()));
            self
        }

        fn with_process(mut self, pid: u32, ppid: u32, name: &str) -> Self {
            self.processes.push(ProcessInfo {
                pid,
                ppid,
                name: name.to_string(),
            });
            self
        }

        fn with_current_pid(mut self, pid: u32) -> Self {
            self.current_pid = pid;
            self
        }
    }

    impl SystemProbe for FakeProbe {
        fn os(&self) -> &str {
            self.os
        }

        fn arch(&self) -> &str {
            self.arch
        }

        fn os_release(&self) -> Option<String> {
            self.os_release.clone()
        }

        fn read_file(&self, path: &str) -> Option<String> {
            self.files
                .iter()
                .find(|(name, _)| name == path)
                .map(|(_, contents)| contents.clone())
        }

        fn path_exists(&self, path: &str) -> bool {
            self.files.iter().any(|(name, _)| name == path)
        }

        fn privilege(&self) -> Privilege {
            self.privilege
        }

        fn stdin_is_terminal(&self) -> bool {
            self.stdin_is_terminal
        }

        fn stdout_is_terminal(&self) -> bool {
            self.stdout_is_terminal
        }

        fn var(&self, key: &str) -> Option<String> {
            self.vars
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }

        fn current_pid(&self) -> u32 {
            self.current_pid
        }

        fn processes(&self) -> Vec<ProcessInfo> {
            self.processes.clone()
        }
    }

    fn var_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    fn parsed_distro(contents: &str) -> Distro {
        match Distro::parse(contents) {
            Some(distro) => distro,
            None => panic!("os-release should parse: {contents:?}"),
        }
    }

    fn detected_app(pairs: &[(&str, &str)]) -> TerminalApp {
        match detect_terminal_app(var_map(pairs)) {
            Some(app) => app,
            None => panic!("terminal application should be detected: {pairs:?}"),
        }
    }

    fn system_distro(system: &SystemInfo) -> &Distro {
        match system.os().distro() {
            Some(distro) => distro,
            None => panic!("system should have a distro"),
        }
    }

    fn system_terminal_app(system: &SystemInfo) -> &TerminalApp {
        match system.terminal().app() {
            Some(app) => app,
            None => panic!("system should have a terminal application"),
        }
    }

    #[test]
    fn arch_parses_rust_and_common_names() {
        assert_eq!(Arch::parse("x86_64"), Arch::X86_64);
        assert_eq!(Arch::parse("amd64"), Arch::X86_64);
        assert_eq!(Arch::parse("aarch64"), Arch::Aarch64);
        assert_eq!(Arch::parse("arm64"), Arch::Aarch64);
        assert_eq!(Arch::parse("i686"), Arch::X86);
        assert_eq!(Arch::parse("armv7l"), Arch::Arm);
        assert_eq!(Arch::parse("riscv64"), Arch::Riscv64);
        assert_eq!(Arch::parse("s390x"), Arch::S390x);
        assert_eq!(Arch::parse("mips"), Arch::Other("mips".to_string()));
    }

    #[test]
    fn arch_names_are_lowercase_and_display_matches() {
        let architectures = [
            Arch::X86,
            Arch::X86_64,
            Arch::Arm,
            Arch::Aarch64,
            Arch::Riscv64,
            Arch::Powerpc,
            Arch::Powerpc64,
            Arch::S390x,
            Arch::Other("mips".to_string()),
        ];
        for arch in architectures {
            assert_eq!(arch.to_string(), arch.as_str());
            assert_eq!(arch.as_str(), arch.as_str().to_lowercase());
        }
    }

    #[test]
    fn current_arch_matches_the_target() {
        assert_eq!(Arch::current(), Arch::parse(std::env::consts::ARCH));
    }

    #[test]
    fn privilege_names_and_elevation() {
        assert_eq!(Privilege::StandardUser.as_str(), "standard");
        assert_eq!(Privilege::Elevated.as_str(), "elevated");
        assert_eq!(Privilege::Unknown.as_str(), "unknown");
        assert!(!Privilege::StandardUser.is_elevated());
        assert!(Privilege::Elevated.is_elevated());
        assert!(!Privilege::Unknown.is_elevated());
        assert_eq!(Privilege::Elevated.to_string(), "elevated");
    }

    #[test]
    fn distro_parses_ubuntu_os_release() {
        let distro =
            parsed_distro("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\n");

        assert_eq!(distro.id(), "ubuntu");
        assert_eq!(distro.name(), Some("Ubuntu"));
        assert_eq!(distro.version(), Some("24.04"));
        assert_eq!(distro.id_like(), ["debian"]);
        assert_eq!(distro.display_name(), "Ubuntu");
    }

    #[test]
    fn distro_parses_unquoted_values_and_ignores_noise() {
        let distro = parsed_distro(
            "# comment\n\nMALFORMED\nID=fedora\nNAME=Fedora Linux\nVERSION_ID=40\nID_LIKE=\"rhel centos\"\n",
        );

        assert_eq!(distro.id(), "fedora");
        assert_eq!(distro.name(), Some("Fedora Linux"));
        assert_eq!(distro.version(), Some("40"));
        assert_eq!(distro.id_like(), ["rhel", "centos"]);
    }

    #[test]
    fn distro_unescapes_double_quoted_values() {
        let distro = parsed_distro("ID=test\nNAME=\"A \\\"quoted\\\" \\$name\"\n");

        assert_eq!(distro.name(), Some("A \"quoted\" $name"));
    }

    #[test]
    fn distro_handles_crlf_and_single_quotes() {
        let distro = parsed_distro("ID='arch'\r\nNAME='Arch Linux'\r\n");

        assert_eq!(distro.id(), "arch");
        assert_eq!(distro.name(), Some("Arch Linux"));
    }

    #[test]
    fn distro_requires_a_non_empty_id() {
        assert_eq!(Distro::parse("NAME=\"No id\"\n"), None);
        assert_eq!(Distro::parse("ID=\n"), None);
        assert_eq!(Distro::parse("ID=\"\"\n"), None);
        assert_eq!(Distro::parse(""), None);
    }

    #[test]
    fn distro_falls_back_to_id_for_display() {
        let distro = parsed_distro("ID=alpine\n");

        assert_eq!(distro.name(), None);
        assert_eq!(distro.version(), None);
        assert!(distro.id_like().is_empty());
        assert_eq!(distro.display_name(), "alpine");
        assert_eq!(distro.to_string(), "alpine");
    }

    #[test]
    fn terminal_app_prefers_the_macos_bundle_id() {
        let app = detected_app(&[
            ("__CFBundleIdentifier", "com.googlecode.iterm2"),
            ("TERM_PROGRAM", "iTerm.app"),
        ]);

        assert_eq!(app.name(), "iTerm2");
        assert_eq!(app.bundle_id(), Some("com.googlecode.iterm2"));
    }

    #[test]
    fn terminal_app_maps_known_bundle_ids() {
        let cases = [
            ("com.apple.Terminal", "Apple Terminal"),
            ("com.googlecode.iterm2", "iTerm2"),
            ("com.raphaelamorim.rio", "Rio"),
            ("com.termius-dmg.mac", "Termius"),
            ("dev.zed.Zed", "Zed"),
            ("dev.zed.Zed-Preview", "Zed Preview"),
            ("com.todesktop.230313mzl4w4u92", "Cursor"),
            ("com.exafunction.windsurf", "Windsurf"),
            ("com.jetbrains.intellij", "IntelliJ IDEA"),
            ("com.jetbrains.intellij.ce", "IntelliJ IDEA CE"),
            ("com.jetbrains.pycharm", "PyCharm"),
            ("com.jetbrains.pycharm.ce", "PyCharm CE"),
            ("com.jetbrains.WebStorm", "WebStorm"),
            ("com.jetbrains.CLion", "CLion"),
            ("com.jetbrains.goland", "GoLand"),
            ("com.jetbrains.rubymine", "RubyMine"),
            ("com.jetbrains.PhpStorm", "PhpStorm"),
            ("com.jetbrains.datagrip", "DataGrip"),
            ("com.jetbrains.rider", "Rider"),
            ("com.jetbrains.AppCode", "AppCode"),
            ("com.jetbrains.rustrover", "RustRover"),
            ("com.jetbrains.fleet", "Fleet"),
            ("com.google.android.studio", "Android Studio"),
            ("com.mitchellh.ghostty", "Ghostty"),
            ("com.microsoft.VSCode", "Visual Studio Code"),
        ];
        for (bundle_id, expected_name) in cases {
            let app = detected_app(&[("__CFBundleIdentifier", bundle_id)]);
            assert_eq!(app.name(), expected_name);
            assert_eq!(app.bundle_id(), Some(bundle_id));
        }
    }

    #[test]
    fn terminal_app_maps_term_program_values() {
        let cases = [
            (
                "Apple_Terminal",
                "Apple Terminal",
                Some("com.apple.Terminal"),
            ),
            ("iTerm.app", "iTerm2", Some("com.googlecode.iterm2")),
            ("rio", "Rio", Some("com.raphaelamorim.rio")),
            ("Termius", "Termius", Some("com.termius-dmg.mac")),
            ("zed", "Zed", Some("dev.zed.Zed")),
            ("cursor", "Cursor", Some("com.todesktop.230313mzl4w4u92")),
            ("windsurf", "Windsurf", Some("com.exafunction.windsurf")),
            ("JetBrains-JediTerm", "JetBrains", None),
        ];
        for (program, expected_name, expected_bundle_id) in cases {
            let app = detected_app(&[("TERM_PROGRAM", program)]);
            assert_eq!(app.name(), expected_name, "unexpected name for {program}");
            assert_eq!(
                app.bundle_id(),
                expected_bundle_id,
                "unexpected bundle id for {program}"
            );
        }
    }

    #[test]
    fn terminal_app_uses_term_program_for_an_unknown_bundle_id() {
        let app = detected_app(&[
            ("__CFBundleIdentifier", "com.example.terminal"),
            ("TERM_PROGRAM", "WezTerm"),
        ]);

        assert_eq!(app.name(), "WezTerm");
        assert_eq!(app.bundle_id(), Some("com.example.terminal"));
    }

    #[test]
    fn terminal_app_falls_back_to_the_bundle_id_as_its_name() {
        let app = detected_app(&[("__CFBundleIdentifier", "com.example.terminal")]);

        assert_eq!(app.name(), "com.example.terminal");
        assert_eq!(app.bundle_id(), Some("com.example.terminal"));
    }

    #[test]
    fn terminal_app_detects_term_program() {
        let app = detected_app(&[("TERM_PROGRAM", "Apple_Terminal")]);

        assert_eq!(app.name(), "Apple Terminal");
        assert_eq!(app.bundle_id(), Some("com.apple.Terminal"));
        assert_eq!(app.to_string(), "Apple Terminal");
    }

    #[test]
    fn terminal_app_keeps_unknown_term_program_names() {
        let app = detected_app(&[("TERM_PROGRAM", "SomeTerm")]);

        assert_eq!(app.name(), "SomeTerm");
        assert_eq!(app.bundle_id(), None);
    }

    #[test]
    fn terminal_app_detects_environment_markers() {
        let cases = [
            ("WT_SESSION", "Windows Terminal", None),
            ("KITTY_WINDOW_ID", "kitty", Some("net.kovidgoyal.kitty")),
            ("WEZTERM_PANE", "WezTerm", Some("com.github.wez.wezterm")),
            ("ALACRITTY_WINDOW_ID", "Alacritty", Some("org.alacritty")),
            (
                "GNOME_TERMINAL_SCREEN",
                "GNOME Terminal",
                Some("org.gnome.Terminal"),
            ),
            ("KONSOLE_VERSION", "Konsole", Some("org.kde.konsole")),
            ("ZED_TERM", "Zed", Some("dev.zed.Zed")),
            (
                "CURSOR_TRACE_ID",
                "Cursor",
                Some("com.todesktop.230313mzl4w4u92"),
            ),
            (
                "WINDSURF_SESSION",
                "Windsurf",
                Some("com.exafunction.windsurf"),
            ),
            (
                "VSCODE_IPC_HOOK_CLI",
                "Visual Studio Code",
                Some("com.microsoft.VSCode"),
            ),
            ("TERMINAL_EMULATOR", "JetBrains", None),
        ];
        for (variable, expected_name, expected_bundle_id) in cases {
            let app = detected_app(&[(variable, "1")]);
            assert_eq!(app.name(), expected_name);
            assert_eq!(app.bundle_id(), expected_bundle_id);
        }
    }

    #[test]
    fn terminal_app_ignores_empty_variables_and_unknown_hosts() {
        assert_eq!(detect_terminal_app(var_map(&[])), None);
        assert_eq!(
            detect_terminal_app(var_map(&[
                ("__CFBundleIdentifier", ""),
                ("TERM_PROGRAM", ""),
                ("WT_SESSION", ""),
            ])),
            None
        );
    }

    #[test]
    fn terminal_app_falls_back_to_the_parent_process_chain_on_linux() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64")
                .with_current_pid(300)
                .with_process(300, 250, "/usr/bin/freshen")
                .with_process(250, 200, "/usr/bin/tmux")
                .with_process(200, 100, "/bin/bash")
                .with_process(100, 1, "/usr/libexec/gnome-terminal-server"),
        );

        let app = system_terminal_app(&system);
        assert_eq!(app.name(), "GNOME Terminal");
        assert_eq!(app.bundle_id(), Some("org.gnome.Terminal"));
    }

    #[test]
    fn terminal_app_falls_back_to_the_parent_process_chain_on_windows() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("windows", "x86_64")
                .with_current_pid(400)
                .with_process(400, 300, "freshen.exe")
                .with_process(300, 200, "powershell.exe")
                .with_process(200, 100, "WindowsTerminal.exe")
                .with_process(100, 0, "explorer.exe"),
        );

        let app = system_terminal_app(&system);
        assert_eq!(app.name(), "Windows Terminal");
        assert_eq!(app.bundle_id(), None);
    }

    #[test]
    fn terminal_app_environment_markers_win_over_the_process_chain() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("windows", "x86_64")
                .with_var("WT_SESSION", "session-id")
                .with_current_pid(400)
                .with_process(400, 300, "freshen.exe")
                .with_process(300, 200, "powershell.exe")
                .with_process(200, 100, "kitty.exe"),
        );

        assert_eq!(
            system.terminal().app().map(TerminalApp::name),
            Some("Windows Terminal")
        );
    }

    #[test]
    fn terminal_app_process_fallback_is_limited_to_linux_and_windows() {
        for os in ["macos", "freebsd"] {
            let system = SystemInfo::detect_with(
                &FakeProbe::new(os, "x86_64")
                    .with_current_pid(300)
                    .with_process(300, 200, "freshen")
                    .with_process(200, 100, "konsole"),
            );

            assert_eq!(
                system.terminal().app(),
                None,
                "{os} must not use the process chain"
            );
        }
    }

    #[test]
    fn terminal_app_process_walk_stops_at_missing_parents_and_cycles() {
        let missing_parent = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64")
                .with_current_pid(10)
                .with_process(10, 20, "freshen"),
        );
        assert_eq!(missing_parent.terminal().app(), None);

        let cycle = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64")
                .with_current_pid(10)
                .with_process(10, 20, "freshen")
                .with_process(20, 10, "bash"),
        );
        assert_eq!(cycle.terminal().app(), None);
    }

    #[test]
    fn terminal_process_names_accept_paths_and_windows_executables() {
        let cases = [
            (
                "/usr/bin/konsole",
                Some(("Konsole", Some("org.kde.konsole"))),
            ),
            (
                "C:/Program Files/WindowsApps/WindowsTerminal.exe",
                Some(("Windows Terminal", None)),
            ),
            (
                "/opt/wezterm/wezterm-gui",
                Some(("WezTerm", Some("com.github.wez.wezterm"))),
            ),
            (
                "/usr/bin/code-insiders",
                Some(("Visual Studio Code", Some("com.microsoft.VSCode"))),
            ),
            (
                "/usr/bin/cursor",
                Some(("Cursor", Some("com.todesktop.230313mzl4w4u92"))),
            ),
            (
                "/usr/bin/windsurf",
                Some(("Windsurf", Some("com.exafunction.windsurf"))),
            ),
            ("/usr/bin/zed", Some(("Zed", Some("dev.zed.Zed")))),
            ("/usr/bin/rio", Some(("Rio", Some("com.raphaelamorim.rio")))),
            (
                "C:/Users/me/AppData/Local/Programs/Termius.exe",
                Some(("Termius", Some("com.termius-dmg.mac"))),
            ),
            (
                "/opt/idea/bin/idea",
                Some(("IntelliJ IDEA", Some("com.jetbrains.intellij"))),
            ),
            (
                "/usr/bin/pycharm",
                Some(("PyCharm", Some("com.jetbrains.pycharm"))),
            ),
            ("/usr/bin/unknown-terminal", None),
        ];

        for (name, expected) in cases {
            let app = terminal_app_for_process(name);
            assert_eq!(
                app.as_ref().map(|app| (app.name(), app.bundle_id())),
                expected,
                "unexpected mapping for {name}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_stat_parses_names_with_spaces_and_parentheses() {
        let stat = "42 (my (odd) shell) S 7 42 42 0 -1 4194304 0 0 0 0 1 2\n";

        match super::parse_proc_stat(stat) {
            Some((ppid, name)) => {
                assert_eq!(ppid, 7);
                assert_eq!(name, "my (odd) shell");
            }
            None => panic!("stat line should parse: {stat:?}"),
        }
        assert_eq!(super::parse_proc_stat("not a stat line"), None);
    }

    #[test]
    fn terminal_requires_both_streams_for_interactive() {
        let cases = [
            (true, true, true),
            (true, false, false),
            (false, true, false),
            (false, false, false),
        ];
        for (stdin_is_terminal, stdout_is_terminal, interactive) in cases {
            let terminal = Terminal {
                stdin_is_terminal,
                stdout_is_terminal,
                app: None,
            };
            assert_eq!(terminal.stdin_is_terminal(), stdin_is_terminal);
            assert_eq!(terminal.stdout_is_terminal(), stdout_is_terminal);
            assert_eq!(terminal.is_interactive(), interactive);
        }
    }

    #[test]
    fn environment_defaults_to_native_on_linux() {
        let system = SystemInfo::detect_with(&FakeProbe::new("linux", "x86_64"));

        let environment = system.environment();
        assert_eq!(environment, &Environment::default());
        assert!(environment.is_native());
        assert!(!environment.is_wsl());
        assert!(!environment.is_container());
        assert_eq!(environment.wsl_distro(), None);
        assert_eq!(system.os().environment(), environment);
    }

    #[test]
    fn environment_detects_wsl_from_distro_name() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64").with_var("WSL_DISTRO_NAME", "Ubuntu-22.04"),
        );

        let environment = system.environment();
        assert!(environment.is_wsl());
        assert!(!environment.is_container());
        assert!(!environment.is_native());
        assert_eq!(environment.wsl_distro(), Some("Ubuntu-22.04"));
    }

    #[test]
    fn environment_detects_wsl_from_proc_version() {
        let system = SystemInfo::detect_with(&FakeProbe::new("linux", "x86_64").with_file(
            "/proc/version",
            "Linux version 5.15.90.1-microsoft-standard-WSL2 (root@build) #1 SMP\n",
        ));

        assert!(system.environment().is_wsl());
        assert_eq!(system.environment().wsl_distro(), None);
    }

    #[test]
    fn environment_detects_wsl_from_interop_marker() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64").with_var("WSL_INTEROP", "/run/WSL/1_interop"),
        );

        assert!(system.environment().is_wsl());
    }

    #[test]
    fn environment_detects_container_marker_files() {
        for path in ["/.dockerenv", "/run/.containerenv"] {
            let system =
                SystemInfo::detect_with(&FakeProbe::new("linux", "x86_64").with_file(path, ""));

            let environment = system.environment();
            assert!(
                environment.is_container(),
                "marker {path} should be detected"
            );
            assert!(!environment.is_wsl());
            assert!(!environment.is_native());
        }
    }

    #[test]
    fn environment_detects_container_environment_variable() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64").with_var("container", "podman"),
        );

        assert!(system.environment().is_container());
    }

    #[test]
    fn environment_detects_container_runtimes_in_cgroup() {
        for marker in ["docker", "containerd", "kubepods", "lxc", "podman"] {
            let cgroup = format!("12:devices:/{marker}/abc123\n");
            let system = SystemInfo::detect_with(
                &FakeProbe::new("linux", "x86_64").with_file("/proc/1/cgroup", &cgroup),
            );

            assert!(
                system.environment().is_container(),
                "cgroup marker {marker} should be detected"
            );
        }
    }

    #[test]
    fn environment_can_be_wsl_and_container_at_once() {
        let system = SystemInfo::detect_with(
            &FakeProbe::new("linux", "x86_64")
                .with_var("WSL_DISTRO_NAME", "Ubuntu")
                .with_file("/.dockerenv", ""),
        );

        assert!(system.environment().is_wsl());
        assert!(system.environment().is_container());
        assert!(!system.environment().is_native());
    }

    #[test]
    fn environment_markers_are_ignored_off_linux() {
        for os in ["macos", "windows"] {
            let system = SystemInfo::detect_with(
                &FakeProbe::new(os, "x86_64")
                    .with_var("WSL_DISTRO_NAME", "Ubuntu")
                    .with_var("container", "podman")
                    .with_file("/.dockerenv", "")
                    .with_file(
                        "/proc/version",
                        "Linux version 5.15.90.1-microsoft-standard-WSL2\n",
                    ),
            );

            assert_eq!(system.environment(), &Environment::default());
        }
    }

    #[test]
    fn detect_with_reads_every_fact_from_the_probe() {
        let probe = FakeProbe::new("linux", "aarch64")
            .with_os_release("ID=debian\nNAME=\"Debian GNU/Linux\"\nVERSION_ID=12\n")
            .with_var("TERM_PROGRAM", "ghostty");
        let mut probe = probe;
        probe.privilege = Privilege::Elevated;
        probe.stdout_is_terminal = false;

        let system = SystemInfo::detect_with(&probe);

        assert_eq!(system.os().name(), "linux");
        assert_eq!(system.platform(), Some(Platform::Linux));
        let distro = system_distro(&system);
        assert_eq!(distro.id(), "debian");
        assert_eq!(distro.version(), Some("12"));
        assert_eq!(system.arch(), &Arch::Aarch64);
        assert_eq!(system.privilege(), Privilege::Elevated);
        assert!(!system.is_interactive());
        assert!(system.terminal().stdin_is_terminal());
        assert!(!system.terminal().stdout_is_terminal());
        let app = system_terminal_app(&system);
        assert_eq!(app.name(), "Ghostty");
    }

    #[test]
    fn detect_with_handles_macos_and_windows() {
        let macos = SystemInfo::detect_with(&FakeProbe::new("macos", "x86_64"));
        assert_eq!(macos.platform(), Some(Platform::Macos));
        assert_eq!(macos.os().distro(), None);

        let windows = SystemInfo::detect_with(
            &FakeProbe::new("windows", "x86_64").with_var("WT_SESSION", "session-id"),
        );
        assert_eq!(windows.platform(), Some(Platform::Windows));
        assert_eq!(
            windows.terminal().app().map(TerminalApp::name),
            Some("Windows Terminal")
        );
    }

    #[test]
    fn detect_with_reports_unsupported_operating_systems() {
        let system = SystemInfo::detect_with(&FakeProbe::new("freebsd", "x86_64"));

        assert_eq!(system.os().name(), "freebsd");
        assert_eq!(system.platform(), None);
    }
}
