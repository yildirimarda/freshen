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

use std::collections::HashMap;
use std::fmt;
use std::io::IsTerminal;
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

fn terminal_name_for_bundle_id(bundle_id: &str) -> Option<&'static str> {
    match bundle_id {
        "com.apple.Terminal" => Some("Apple Terminal"),
        "com.googlecode.iterm2" => Some("iTerm2"),
        "com.mitchellh.ghostty" => Some("Ghostty"),
        "com.microsoft.VSCode" => Some("Visual Studio Code"),
        "com.github.wez.wezterm" => Some("WezTerm"),
        "net.kovidgoyal.kitty" => Some("kitty"),
        "org.alacritty" => Some("Alacritty"),
        "co.zeit.hyper" => Some("Hyper"),
        "dev.warp.Warp-Stable" => Some("Warp"),
        _ => None,
    }
}

fn terminal_name_for_program(program: &str) -> &str {
    match program {
        "Apple_Terminal" => "Apple Terminal",
        "iTerm.app" => "iTerm2",
        "vscode" => "Visual Studio Code",
        "ghostty" => "Ghostty",
        "WezTerm" => "WezTerm",
        "kitty" => "kitty",
        "Hyper" => "Hyper",
        "WarpTerminal" => "Warp",
        "Tabby" => "Tabby",
        other => other,
    }
}

fn bundle_id_for_program(program: &str) -> Option<&'static str> {
    match program {
        "Apple_Terminal" => Some("com.apple.Terminal"),
        "iTerm.app" => Some("com.googlecode.iterm2"),
        "vscode" => Some("com.microsoft.VSCode"),
        "ghostty" => Some("com.mitchellh.ghostty"),
        "WezTerm" => Some("com.github.wez.wezterm"),
        "kitty" => Some("net.kovidgoyal.kitty"),
        "Hyper" => Some("co.zeit.hyper"),
        "WarpTerminal" => Some("dev.warp.Warp-Stable"),
        _ => None,
    }
}

fn detect_terminal_app(get: impl Fn(&str) -> Option<String>) -> Option<TerminalApp> {
    let bundle_id = get("__CFBundleIdentifier").filter(|value| !value.is_empty());
    let term_program = get("TERM_PROGRAM").filter(|value| !value.is_empty());

    if let Some(bundle_id) = bundle_id {
        let name = terminal_name_for_bundle_id(&bundle_id)
            .map(str::to_string)
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

    let markers = [
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
        (
            "VSCODE_IPC_HOOK_CLI",
            "Visual Studio Code",
            Some("com.microsoft.VSCode"),
        ),
        ("TERMINAL_EMULATOR", "JetBrains", None),
    ];

    for (variable, name, bundle_id) in markers {
        if get(variable).is_some_and(|value| !value.is_empty()) {
            return Some(TerminalApp::new(name, bundle_id.map(str::to_string)));
        }
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
            app: detect_terminal_app(|key| probe.var(key)),
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
        detect_terminal_app, Arch, Distro, Environment, Privilege, SystemInfo, SystemProbe,
        Terminal, TerminalApp,
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
            ("WT_SESSION", "Windows Terminal"),
            ("KITTY_WINDOW_ID", "kitty"),
            ("WEZTERM_PANE", "WezTerm"),
            ("ALACRITTY_WINDOW_ID", "Alacritty"),
            ("GNOME_TERMINAL_SCREEN", "GNOME Terminal"),
            ("KONSOLE_VERSION", "Konsole"),
            ("VSCODE_IPC_HOOK_CLI", "Visual Studio Code"),
            ("TERMINAL_EMULATOR", "JetBrains"),
        ];
        for (variable, expected_name) in cases {
            let app = detected_app(&[(variable, "1")]);
            assert_eq!(app.name(), expected_name);
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
