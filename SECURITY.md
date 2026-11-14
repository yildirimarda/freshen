# Security Policy

## Supported Versions

freshen is a single static binary. Security fixes are released for the latest
stable version only. Older versions do not receive security updates, so keep
freshen up to date through its self-update check or your package manager.

| Version | Supported |
| ------- | --------- |
| Latest  | :white_check_mark: |
| < Latest | :x: |

## Reporting a Vulnerability

Please do not open a public GitHub issue for a security vulnerability.

Report it through GitHub's private vulnerability reporting: open the
"Security" tab of this repository, then "Report a vulnerability", or send the
details to the repository's security advisory draft. Include as much as you
can of the following:

- The freshen version (`freshen --version`)
- Your operating system and version (macOS, Linux distribution, Windows)
- Which steps you use (Homebrew, uv, mise, Docker/Colima, apt, winget, ...)
- A clear description of the vulnerability and how to reproduce it
- The command you ran and its complete output (`freshen --doctor` output is
  especially useful)

Redact any paths, hostnames or credentials you do not want to share before you
paste command output.

## What to Expect

- You will receive an acknowledgement within 7 days.
- We will investigate and keep you informed of the progress.
- If the report is confirmed, we will fix it, publish a release and credit
  you (unless you ask to remain anonymous).
- We follow coordinated disclosure: details are made public only after a fix
  is available.

## Security Considerations

- freshen only runs the package-manager and toolchain commands you ask it to
  run (brew, apt, dnf, winget, Chocolatey, Scoop, uv, mise, Docker, ...). It
  never downloads or executes code from the internet on its own, apart from
  the optional self-update check, which you can disable in the config file.
- freshen collects no telemetry and sends no usage data anywhere.
- freshen never writes to the system outside the locations the package
  managers themselves manage, and destructive cleanup (cache pruning,
  autoremove, image prune) only runs with the explicit `--cleanup` flag.
- Runs are performed as the invoking user. On Linux and macOS, freshen refuses
  to run user-level steps (uv, mise, npm, gem, pipx) as root. On Windows,
  steps that need administrator rights are skipped with a clear reason unless
  the run is elevated.

## Disclosure Policy

We accept vulnerability reports under the GitHub Security Advisory process.
Public disclosure of a vulnerability before we have had a reasonable chance to
fix it is not in keeping with this policy. If you believe a report has not
been handled appropriately, re-open the advisory and we will re-engage.
