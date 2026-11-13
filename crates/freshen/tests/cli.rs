//! End-to-end checks for the `freshen` binary's command-line surface.
//!
//! These tests never run a package manager: detection only looks for an
//! executable on `PATH`, and the run cases exclude every built-in step.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory without a configuration file, so the binary never reads the
/// configuration of the machine the test runs on.
fn empty_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "freshen-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", home.display()));
    home
}

fn run(args: &[&str]) -> Output {
    let home = empty_home();
    match Command::new(env!("CARGO_BIN_EXE_freshen"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("APPDATA", home.join("appdata"))
        .args(args)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("failed to run freshen: {error}"),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The built-in steps that exist on every platform.
const COMMON_STEPS: &str = "antidote,aws,az,brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-doctor,brew-bundle-cleanup,brew-reopen,colima-stop,colima-start,docker,dotnet-tools,go,rustup,asdf,cargo-update,composer,uv,mise,fnm,gem,gh,gcloud,helm,git-lfs,pre-commit,npm,nvm,volta,pnpm,pip,pipx,vscode,cursor,vscodium,pacman,aur,zypper,apk,emerge,termux,firmware,fisher,flatpak,sdkman,snap,nix,nix-profile,nix-collect-garbage,home-manager,rpm-ostree,bootc,microsoft-store,yarn,bun,pyenv,rbenv,nvim-lazy,nvim-packer,nvim-mason,nvim-treesitter,oh-my-zsh";

/// The built-in steps that only exist on macOS.
const MACOS_STEPS: &str = "mas,macos-updates";

#[test]
fn help_lists_every_flag() {
    let output = run(&["--help"]);

    assert!(output.status.success());
    let text = stdout(&output);
    for flag in [
        "--dry-run",
        "--doctor",
        "--check",
        "--list-steps",
        "--json",
        "--markdown",
        "--config",
        "--verbose",
        "--quiet",
        "--container-runtime",
        "--only",
        "--skip",
        "--profile",
        "--yes",
        "--force-kill",
        "--step-timeout",
        "--timeout",
        "--skip-network-check",
        "--skip-disk-check",
        "--skip-battery-check",
        "--skip-metered-check",
        "--min-free-disk-gib",
        "--battery-threshold",
        "--install-aws-cli-updates",
    ] {
        assert!(text.contains(flag), "help does not mention {flag}: {text}");
    }
}

#[test]
fn help_documents_the_stable_exit_codes() {
    let output = run(&["--help"]);

    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("Exit codes:"), "help: {text}");
    for line in [
        "0    success",
        "1    a step failed or timed out",
        "2    updates are pending (--check)",
        "3    a preflight or package-manager check failed",
        "130  the run was interrupted",
    ] {
        assert!(
            text.contains(line),
            "help does not mention `{line}`: {text}"
        );
    }
}

#[test]
fn list_steps_lists_every_builtin_step() {
    let output = run(&["--list-steps"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Steps (84):"), "listing: {text}");
    assert!(text.contains("antidote"), "listing: {text}");
    assert!(text.contains("aws"), "listing: {text}");
    assert!(text.contains("az"), "listing: {text}");
    assert!(text.contains("conda"), "listing: {text}");
    assert!(text.contains("mamba"), "listing: {text}");
    assert!(text.contains("fnm"), "listing: {text}");
    assert!(text.contains("gem"), "listing: {text}");
    assert!(text.contains("gcloud"), "listing: {text}");
    assert!(text.contains("go"), "listing: {text}");
    assert!(text.contains("git-lfs"), "listing: {text}");
    assert!(text.contains("pre-commit"), "listing: {text}");
    assert!(text.contains("wsl"), "listing: {text}");
    assert!(text.contains("npm"), "listing: {text}");
    assert!(text.contains("nvm"), "listing: {text}");
    assert!(text.contains("nvim-lazy"), "listing: {text}");
    assert!(text.contains("nvim-packer"), "listing: {text}");
    assert!(text.contains("nvim-mason"), "listing: {text}");
    assert!(text.contains("nvim-treesitter"), "listing: {text}");
    assert!(text.contains("oh-my-zsh"), "listing: {text}");
    assert!(text.contains("zinit"), "listing: {text}");
    assert!(text.contains("tpm"), "listing: {text}");
    assert!(text.contains("volta"), "listing: {text}");
    assert!(text.contains("pnpm"), "listing: {text}");
    assert!(text.contains("brew"), "listing: {text}");
    assert!(text.contains("brew-casks"), "listing: {text}");
    assert!(text.contains("brew-doctor"), "listing: {text}");
    assert!(text.contains("brew-bundle-cleanup"), "listing: {text}");
    assert!(text.contains("colima-stop"), "listing: {text}");
    assert!(text.contains("docker"), "listing: {text}");
    assert!(text.contains("rustup"), "listing: {text}");
    assert!(text.contains("asdf"), "listing: {text}");
    assert!(text.contains("cargo-update"), "listing: {text}");
    assert!(text.contains("composer"), "listing: {text}");
    assert!(text.contains("dotnet-tools"), "listing: {text}");
    assert!(text.contains("pyenv"), "listing: {text}");
    assert!(text.contains("rbenv"), "listing: {text}");
    assert!(text.contains("uv"), "listing: {text}");
    assert!(text.contains("mise"), "listing: {text}");
    assert!(text.contains("pip"), "listing: {text}");
    assert!(text.contains("pipx"), "listing: {text}");
    assert!(text.contains("vscode"), "listing: {text}");
    assert!(text.contains("cursor"), "listing: {text}");
    assert!(text.contains("vscodium"), "listing: {text}");
    assert!(text.contains("gh"), "listing: {text}");
    assert!(text.contains("kubectl"), "listing: {text}");
    assert!(text.contains("mas"), "listing: {text}");
    assert!(text.contains("macos-updates"), "listing: {text}");
    assert!(text.contains("apt"), "listing: {text}");
    assert!(text.contains("termux"), "listing: {text}");
    assert!(text.contains("dnf"), "listing: {text}");
    assert!(text.contains("pacman"), "listing: {text}");
    assert!(text.contains("aur"), "listing: {text}");
    assert!(text.contains("zypper"), "listing: {text}");
    assert!(text.contains("apk"), "listing: {text}");
    assert!(text.contains("emerge"), "listing: {text}");
    assert!(text.contains("firmware"), "listing: {text}");
    assert!(text.contains("flatpak"), "listing: {text}");
    assert!(text.contains("helm"), "listing: {text}");
    assert!(text.contains("helm-releases"), "listing: {text}");
    assert!(text.contains("sdkman"), "listing: {text}");
    assert!(text.contains("jenv"), "listing: {text}");
    assert!(text.contains("tfenv"), "listing: {text}");
    assert!(text.contains("snap"), "listing: {text}");
    assert!(text.contains("nix"), "listing: {text}");
    assert!(text.contains("nix-profile"), "listing: {text}");
    assert!(text.contains("nix-collect-garbage"), "listing: {text}");
    assert!(text.contains("home-manager"), "listing: {text}");
    assert!(text.contains("rpm-ostree"), "listing: {text}");
    assert!(text.contains("winget"), "listing: {text}");
    assert!(text.contains("chocolatey"), "listing: {text}");
    assert!(text.contains("microsoft-store"), "listing: {text}");
    assert!(text.contains("scoop"), "listing: {text}");
    assert!(text.contains("windows-update"), "listing: {text}");
    assert!(text.contains("yarn"), "listing: {text}");
    assert!(text.contains("bun"), "listing: {text}");
    assert!(text.contains("elixir"), "listing: {text}");
}

#[test]
fn version_prints_the_binary_name_and_version() {
    let output = run(&["--version"]);

    assert!(output.status.success());
    assert!(stdout(&output).starts_with("freshen "));
}

#[test]
fn logs_shows_the_last_run() {
    let home = empty_home();

    let env = |command: &mut Command| {
        command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("APPDATA", home.join("appdata"));
    };

    let run = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_freshen"));
        env(&mut command);
        command
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run freshen: {error}"))
    };

    // Produce a run whose events are recorded in the log file.
    let first = run(&[
        "--dry-run",
        "--skip",
        &format!("{COMMON_STEPS},{MACOS_STEPS}"),
    ]);
    assert_eq!(first.status.code(), Some(0), "stderr: {}", stderr(&first));

    // The `logs` subcommand prints the last run from the log file.
    let logged = run(&["logs"]);
    assert_eq!(logged.status.code(), Some(0), "stderr: {}", stderr(&logged));
    let text = stdout(&logged);
    assert!(text.contains("started"), "logs output: {text}");
    assert!(text.contains("run completed"), "logs output: {text}");
}

#[test]
fn logs_with_no_previous_run_prints_nothing() {
    let home = empty_home();

    let output = Command::new(env!("CARGO_BIN_EXE_freshen"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("APPDATA", home.join("appdata"))
        .env("LOCALAPPDATA", home.join("localappdata"))
        .args(["logs"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(stdout(&output).is_empty(), "stdout: {:?}", stdout(&output));
}

#[test]
fn logs_subcommand_is_listed_in_help() {
    let output = run(&["--help"]);

    assert!(output.status.success());
    let text = stdout(&output);
    assert!(
        text.contains("logs"),
        "help does not mention the logs subcommand: {text}"
    );
}

#[test]
fn history_lists_previous_runs() {
    let home = empty_home();
    let snapshot_dir = home.join("snapshots");
    std::fs::create_dir_all(&snapshot_dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", snapshot_dir.display()));

    // Newest run first, by filename timestamp.
    std::fs::write(
        snapshot_dir.join("snapshot-2026-07-20T22-13-20Z-1.txt"),
        "# freshen pre-upgrade snapshot\n\
         # created: 2026-07-20T22:13:20Z\n\
         # freshen: 0.1.0\n\
         \n\
         ## brew\n\
         \n\
         $ brew list --versions\n\
         jq 1.7.1\n\
         python@3.11 3.11.6\n\
         \n\
         ## uv\n\
         \n\
         $ uv tool list\n\
         ruff v0.6.0\n",
    )
    .unwrap_or_else(|error| panic!("failed to write snapshot: {error}"));
    std::fs::write(
        snapshot_dir.join("snapshot-2026-07-19T10-00-00Z-1.txt"),
        "# freshen pre-upgrade snapshot\n\
         # created: 2026-07-19T10:00:00Z\n\
         # freshen: 0.1.0\n\
         \n\
         ## brew\n\
         \n\
         $ brew list --versions\n\
         jq 1.7.0\n",
    )
    .unwrap_or_else(|error| panic!("failed to write snapshot: {error}"));

    let output = Command::new(env!("CARGO_BIN_EXE_freshen"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("APPDATA", home.join("appdata"))
        .env("FRESHEN_SNAPSHOT_DIR", &snapshot_dir)
        .args(["history"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Previous runs"), "history output: {text}");
    assert!(
        text.contains("2026-07-20T22:13:20Z"),
        "history output: {text}"
    );
    assert!(
        text.contains("2026-07-19T10:00:00Z"),
        "history output: {text}"
    );
    assert!(text.contains("## brew"), "history output: {text}");
    assert!(text.contains("## uv"), "history output: {text}");
    assert!(
        text.contains("$ brew list --versions"),
        "history output: {text}"
    );
    assert!(text.contains("$ uv tool list"), "history output: {text}");
    assert!(text.contains("jq 1.7.1"), "history output: {text}");
    assert!(
        text.contains("python@3.11 3.11.6"),
        "history output: {text}"
    );
    assert!(text.contains("ruff v0.6.0"), "history output: {text}");
    // Newest run is listed first.
    let newest = text
        .find("2026-07-20T22:13:20Z")
        .unwrap_or_else(|| panic!("newest run not in history output: {text}"));
    let older = text
        .find("2026-07-19T10:00:00Z")
        .unwrap_or_else(|| panic!("older run not in history output: {text}"));
    assert!(newest < older, "newest run should be listed first: {text}");
}

#[test]
fn history_with_no_previous_run_prints_nothing() {
    let home = empty_home();
    let snapshot_dir = home.join("snapshots");
    std::fs::create_dir_all(&snapshot_dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", snapshot_dir.display()));

    let output = Command::new(env!("CARGO_BIN_EXE_freshen"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("APPDATA", home.join("appdata"))
        .env("FRESHEN_SNAPSHOT_DIR", &snapshot_dir)
        .args(["history"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run freshen: {error}"));

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(stdout(&output).is_empty(), "stdout: {:?}", stdout(&output));
}

#[test]
fn history_subcommand_is_listed_in_help() {
    let output = run(&["--help"]);

    assert!(output.status.success());
    let text = stdout(&output);
    assert!(
        text.contains("history"),
        "help does not mention the history subcommand: {text}"
    );
}

#[test]
fn dry_run_with_every_builtin_step_excluded_plans_nothing() {
    let output = run(&[
        "--dry-run",
        "--skip",
        &format!("{COMMON_STEPS},{MACOS_STEPS},apt,dnf,cargo-update,composer,conda,mamba,pnpm,yarn,bun,elixir,gh,gcloud,helm,git-lfs,kubectl,winget,chocolatey,scoop,windows-update,wsl,jenv,tfenv,nvim-lazy,nvim-packer,nvim-mason,nvim-treesitter,oh-my-zsh,zinit,tpm,helm-releases"),
    ]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    // A step that `--skip` excludes is not part of the run at all, so it is
    // neither planned nor counted as skipped. That is the same on every
    // platform: the macOS-only steps are excluded here as well.
    assert!(
        stdout(&output).contains("0 ok, 0 planned, 0 skipped, 0 failed"),
        "summary: {}",
        stdout(&output)
    );
}

#[test]
fn check_with_every_builtin_step_excluded_reports_up_to_date() {
    let output = run(&[
        "--check",
        "--skip",
        &format!("{COMMON_STEPS},{MACOS_STEPS},apt,dnf,cargo-update,composer,conda,mamba,pnpm,yarn,bun,elixir,gh,gcloud,helm,git-lfs,kubectl,winget,chocolatey,scoop,windows-update,wsl,jenv,tfenv,nvim-lazy,nvim-packer,nvim-mason,nvim-treesitter,oh-my-zsh,zinit,tpm,helm-releases"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "Everything is up to date.\n");
}

#[test]
fn doctor_reports_the_environment_and_exits_zero_when_healthy() {
    let output = run(&["--doctor"]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("freshen doctor"),
        "doctor output: {}",
        stdout(&output)
    );
}

#[test]
fn yes_and_force_kill_are_accepted_with_dry_run() {
    let output = run(&["--yes", "--force-kill", "--dry-run"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
}

#[test]
fn unknown_only_step_fails_with_a_clear_message() {
    let output = run(&["--only", "nope"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("unknown step `nope` in --only"));
}

#[test]
fn unknown_skip_step_fails_with_a_clear_message() {
    let output = run(&["--skip", "nope"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("unknown step `nope` in --skip"));
}

/// Writes `text` to a configuration file in `home` and returns its path.
fn write_config(home: &Path, text: &str) -> PathBuf {
    let path = home.join("config.toml");
    std::fs::write(&path, text).unwrap_or_else(|error| panic!("failed to write {path:?}: {error}"));
    path
}

/// Returns the `brew-casks` line of a `--list-steps` output.
/// Returns the `apt` line of a `--list-steps` output, or `None` when apt is
/// not known to the listing.
fn apt_line(text: &str) -> Option<&str> {
    text.lines().find(|line| line.contains("apt"))
}

/// Returns `true` when `apt` is detected as installed on this machine.
fn apt_is_installed(text: &str) -> bool {
    apt_line(text)
        .map(|line| line.contains("installed") && !line.contains("not installed"))
        .unwrap_or(false)
}

#[test]
fn a_profile_disables_a_step_it_names() {
    let home = empty_home();
    let config = write_config(&home, "[profiles.work]\napt = false\n");
    let config = config
        .to_str()
        .unwrap_or_else(|| panic!("config path is not valid UTF-8"));

    let baseline = run(&["--config", config, "--list-steps"]);
    assert_eq!(
        baseline.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&baseline)
    );
    let apt_installed = apt_is_installed(&stdout(&baseline));

    let with_profile = run(&["--config", config, "--profile", "work", "--list-steps"]);
    assert_eq!(
        with_profile.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&with_profile)
    );
    let text = stdout(&with_profile);

    if apt_installed {
        assert!(
            apt_line(&text)
                .map(|line| line.contains("disabled in the configuration file"))
                .unwrap_or(false),
            "listing: {text}"
        );
    } else {
        // On machines without apt the profile is still accepted and applied;
        // nothing should reference an unknown profile.
        assert!(!text.contains("unknown profile"), "listing: {text}");
    }
}

#[test]
fn an_unknown_profile_fails_with_a_clear_message() {
    let output = run(&["--profile", "nope"]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("unknown profile `nope`"),
        "stderr: {}",
        stderr(&output)
    );
}
