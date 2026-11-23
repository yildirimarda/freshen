#!/usr/bin/env python3
"""Validate freshen JSON reports from the read-only real-tool CI jobs.

The jobs must fail on parser drift, not only on a non-zero command exit. Some
parsers (for example `brew outdated`) are deliberately lenient, so a changed
line can become a bogus package name instead of an error; this validator
rejects names that are not package tokens and also fails when an expected step
was skipped, failed, timed out or is missing from the report.

The `--profile` selects the steps a job must exercise: `homebrew` (the default,
so older invocations keep working), `apt` or `windows` (winget and Chocolatey).
"""

import argparse
import json
import re
import sys

PROFILES = {
    "homebrew": {
        "steps": (
            "brew",
            "brew-formulae",
            "brew-casks",
            "brew-autoremove",
            "brew-cleanup",
            "brew-doctor",
            "brew-bundle-cleanup",
            "brew-reopen",
        ),
        "tool": "Homebrew",
        "noun": "Homebrew token",
    },
    "apt": {
        "steps": ("apt",),
        "tool": "apt",
        "noun": "apt package name",
    },
    "dnf": {
        "steps": ("dnf",),
        "tool": "dnf",
        "noun": "dnf package name",
    },
    "pacman": {
        "steps": ("pacman",),
        "tool": "pacman",
        "noun": "pacman package name",
    },
    "windows": {
        "steps": ("winget", "chocolatey"),
        "tool": "winget or Chocolatey",
        "noun": "winget or Chocolatey package name",
    },
}

# Package ids/names seen in these tools' output. This is intentionally a
# little wider than any one tool (apt allows `:` for an architecture and `+`
# in names, winget ids contain dots, Homebrew allows `@` and `/`): it must
# reject prose that leaked into a name, not enforce each tool's exact grammar.
PACKAGE_TOKEN = re.compile(r"^[A-Za-z0-9@._+~:/-]+$")

# Steps whose items are findings with free text, not package names.
FINDING_STEPS = {"brew-doctor"}

# Steps whose item names are product display names (`Google Chrome`,
# `PowerShell 7-x64`), which winget prints in its `Name` column: they contain
# spaces and punctuation, and a long one is cut with `…`.
DISPLAY_NAME_STEPS = {"winget"}

# What a parser that lost its place in the winget table would turn into an
# item: a header word, a dashed line or the `N upgrades available.` footer.
DISPLAY_NAME_DRIFT = re.compile(
    r"^(?:Name|Id|Version|Available|Source|-{3,}|\d+ upgrades? available\.?)$",
    re.IGNORECASE,
)


def load_report(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def steps_by_id(report):
    steps = report.get("steps")
    if not isinstance(steps, list):
        return None, ["report has no steps list"]
    by_id = {}
    errors = []
    for step in steps:
        if not isinstance(step, dict) or not isinstance(step.get("id"), str):
            errors.append("report contains a malformed step")
            continue
        by_id[step["id"]] = step
    return by_id, errors


def validate_step_error(step_id, step, errors):
    if step.get("error") is not None:
        errors.append(f"{step_id}: report contains an error: {step['error']}")


def validate_item(step_id, item, errors, noun):
    if not isinstance(item, dict):
        errors.append(f"{step_id}: malformed outdated item")
        return

    name = item.get("name")
    if step_id in FINDING_STEPS:
        # The items of a report-only step are findings (`brew doctor`
        # warnings), whose names are sentences, not package names.
        if not isinstance(name, str) or not name.strip():
            errors.append(f"{step_id}: finding has no text (parser drift?)")
    elif step_id in DISPLAY_NAME_STEPS:
        if (
            not isinstance(name, str)
            or not name
            or name != name.strip()
            or not name.isprintable()
            or DISPLAY_NAME_DRIFT.fullmatch(name)
        ):
            errors.append(
                f"{step_id}: outdated item {name!r} does not look like a "
                "package display name (parser drift?)"
            )
    elif not isinstance(name, str) or not PACKAGE_TOKEN.fullmatch(name):
        errors.append(
            f"{step_id}: outdated item {name!r} does not look like a {noun} "
            "(parser drift?)"
        )

    for field in ("current", "latest"):
        value = item.get(field)
        if value is None:
            continue
        if not isinstance(value, str) or not value or any(ch.isspace() for ch in value):
            errors.append(f"{step_id}: invalid {field} version {value!r}")


def validate_check(report, profile):
    errors = []
    if report.get("failed"):
        errors.append(f"check report has {report['failed']} failed step(s)")
    if report.get("timed_out"):
        errors.append(f"check report has {report['timed_out']} timed-out step(s)")

    by_id, step_errors = steps_by_id(report)
    errors.extend(step_errors)
    if by_id is None:
        return errors

    for step_id in profile["steps"]:
        step = by_id.get(step_id)
        if step is None:
            errors.append(f"{step_id}: step is missing from the check report")
            continue

        status = step.get("status")
        if status not in ("up to date", "outdated"):
            errors.append(
                f"{step_id}: check status is {status!r}; expected 'up to date' or "
                f"'outdated' (a skipped step means real {profile['tool']} was not "
                "exercised)"
            )
        validate_step_error(step_id, step, errors)

        items = step.get("items")
        if not isinstance(items, list):
            errors.append(f"{step_id}: report has no items list")
            continue
        for item in items:
            validate_item(step_id, item, errors, profile["noun"])

    return errors


def validate_dry_run(report, profile):
    errors = []
    if report.get("failed"):
        errors.append(f"dry-run report has {report['failed']} failed step(s)")
    if report.get("timed_out"):
        errors.append(f"dry-run report has {report['timed_out']} timed-out step(s)")
    if report.get("interrupted") or report.get("was_interrupted"):
        errors.append("dry-run report says the run was interrupted")

    by_id, step_errors = steps_by_id(report)
    errors.extend(step_errors)
    if by_id is None:
        return errors

    for step_id in profile["steps"]:
        step = by_id.get(step_id)
        if step is None:
            errors.append(f"{step_id}: step is missing from the dry-run report")
            continue

        status = step.get("status")
        if status != "planned":
            errors.append(
                f"{step_id}: dry-run status is {status!r}; expected 'planned' "
                f"(a skipped step means real {profile['tool']} was not exercised)"
            )
        validate_step_error(step_id, step, errors)

    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("check", "dry-run"))
    parser.add_argument("report", help="path to the JSON report written by freshen")
    parser.add_argument(
        "--profile",
        choices=tuple(PROFILES),
        default="homebrew",
        help="real tools whose steps the report must contain (default: homebrew)",
    )
    args = parser.parse_args(argv)

    # Item names can hold characters outside the Windows console code page
    # (winget cuts long names with `…`); never let printing them fail or
    # turn into `�` in the job log.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")

    try:
        report = load_report(args.report)
    except (OSError, json.JSONDecodeError) as error:
        print(f"error: cannot read {args.report}: {error}", file=sys.stderr)
        return 1

    if not isinstance(report, dict):
        print(f"error: {args.report} is not a JSON object", file=sys.stderr)
        return 1

    profile = PROFILES[args.profile]
    errors = (
        validate_check(report, profile)
        if args.mode == "check"
        else validate_dry_run(report, profile)
    )
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"ok: {args.mode} report {args.report} ({args.profile})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
