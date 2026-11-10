//! End-to-end checks for the `freshen` binary's command-line surface.
//!
//! These tests never run a package manager: detection only looks for an
//! executable on `PATH`, and the run cases exclude every built-in step.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A directory without a configuration file, so the binary never reads the
/// configuration of the machine the test runs on.
fn empty_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("freshen-cli-{}", std::process::id()));
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
        "--check",
        "--list-steps",
        "--json",
        "--markdown",
        "--config",
        "--container-runtime",
        "--only",
        "--skip",
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
