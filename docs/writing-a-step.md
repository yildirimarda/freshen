# Writing a step

This guide walks through adding a new built-in step to freshen, with a
worked example you can copy and a step template to start from.

A step is one independently runnable unit of work: detect a tool, report what
is outdated, upgrade it, and optionally clean up afterwards. Steps are written
in Rust in `crates/freshen-lib/src/`, one module per step, and are registered
in `crates/freshen-lib/src/builtin_steps.rs`. The `freshen` binary is a thin
CLI wrapper around this library.

If all you need is a one-off command without touching the code, use a
[custom step](config.md#custom-steps) from `config.toml` instead. Write a
built-in step when the tool deserves first-class support: read-only
`--check` reporting, snapshot/rollback hints, ordering against other steps,
platform filters, and restart/restore handling.

## The step contract

Every step implements [`crate::step::Step`], an object-safe trait (steps are
kept behind `Box<dyn Step>` in the registry). A step answers these questions:

| Trait method | Question it answers |
|---|---|
| `id` | Stable step id, e.g. `"brew"` |
| `display_name` | Name shown to the user, e.g. `"Homebrew"` |
| `detect` | Is the tool installed? |
| `check` | What is outdated? Read-only, safe for `--check` mode |
| `run` | Perform the upgrade |
| `cleanup` | Remove leftovers after a successful run (only with `--cleanup`) |

And a few declarations:

| Trait method | What it declares |
|---|---|
| `platforms` | Operating systems the step runs on (`macos`, `linux`, `windows`) |
| `prerequisites` | Step ids that must run before this one |
| `conflicts` | Step ids that must not run in the same run |
| `needs_root` / `needs_administrator` | Whether the step needs elevation |
| `is_user_level` | Whether the step must never run as root |

Optional overrides the runner calls when it needs them:

* `check_with_options` — apply the run's include/exclude lists to the report.
* `run_with_options` — honour `--yes` and `--force-kill` (prompting, killing apps).
* `restore` — restore anything the step stopped (apps, containers, VMs) after
  an interrupt; called in reverse execution order, safe to call when nothing
  was stopped.
* `snapshot` — read-only commands that record installed versions before the
  step runs, for rollback hints.

### Detection

```rust
pub enum Detection {
    Installed,            // the tool is present; the step can run
    NotInstalled,         // the step is skipped
    Unavailable(SkipReason), // the tool is present but cannot run right now
}
```

`detect` must not change the system. Return `NotInstalled` when the tool is
absent — that is the normal, expected case on machines without the tool, and
it is not an error.

### Check

`check` returns a [`CheckReport`] of [`OutdatedItem`]s. An empty report means
"up to date". `check` must be read-only: no installs, no removals, no writes.

```rust
pub struct CheckReport { /* ... */ }

impl CheckReport {
    pub fn up_to_date() -> Self;
    pub fn from_items(items: impl IntoIterator<Item = OutdatedItem>) -> Self;
    pub fn filtered<F>(self, predicate: F) -> Self;
    pub fn items(&self) -> &[OutdatedItem];
    pub fn is_up_to_date(&self) -> bool;
    pub fn pending(&self) -> usize;
}

pub struct OutdatedItem { /* name, current, latest, reason */ }

impl OutdatedItem {
    pub fn new(name: impl Into<String>) -> Self;
    pub fn with_current(mut self, version: impl Into<String>) -> Self;
    pub fn with_latest(mut self, version: impl Into<String>) -> Self;
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self;
}
```

Steps that apply an include or exclude list override `check_with_options` and
call `report.filtered(|name| ...)`; the default ignores the options and calls
`check`.

### Errors

Step failures are [`StepError`], tagged with the step id and the phase
(`Operation::Detect`, `Check`, `Run`, `Cleanup`). Failures are collected — one
failing step never aborts the run — and listed in the summary (exit code 1).

```rust
pub struct StepError { /* step, operation, message */ }

impl StepError {
    pub fn new(step, operation, message) -> Self;
    pub fn run_from_process(step, error: &ProcessError) -> Self;
    pub fn check_from_process(step, error: &ProcessError) -> Self;
    pub fn with_skip_reason(mut self, reason: SkipReason) -> Self;
}
```

Attach [`SkipReason::RegistryUnreachable`] (detected automatically from network
errors) or `NotSignedIn`, `NeedsSudo`, `NeedsAdministrator`,
`MissingConfiguration`, `NothingToDo`, etc. when the step should be reported
as skipped instead of failed.

### Where steps live

* `crates/freshen-lib/src/<step>.rs` — the module, e.g. `uv.rs`, `rustup.rs`.
* `crates/freshen-lib/src/builtin_steps.rs` — `register_all` registers every
  built-in step, in run order.
* `docs/steps/<id>.md` — one generated page per step; re-run
  `cargo run --example generate_step_docs` after adding, renaming or
  re-platforming a step, then add the page to `mkdocs.yml`.

## Step template

Copy this skeleton into `crates/freshen-lib/src/<step>.rs`, replacing `<step>`,
`<name>` and `<id>`:

```rust
//! A one-line description of what the step does.
//!
//! More context: what commands it runs, when it is skipped, platform notes.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{SnapshotCommand, SnapshotSection};
use crate::step::{
    CheckReport, Detection, Operation, OutdatedItem, Step, StepError, StepId,
};

/// Stable step id, used by prerequisites, conflicts and `--only`/`--skip`.
pub const ID: StepId = "<id>";

/// Platforms the step runs on.
const PLATFORMS: [Platform; 3] =
    [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the step. The name of this function does not matter.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(<Step>)
}

struct <Step>;

impl <Step> {
    /// Runs the upgrade, collecting the first failure.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("<step>: upgrade");
        if let Err(error) = CommandSpec::new("<tool>")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(ID, &error));
        }
    }
}

impl Step for <Step> {
    fn id(&self) -> StepId {
        ID
    }

    fn display_name(&self) -> &str {
        "<name>"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_<step>())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("<tool>")
            .args(["--outdated"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(ID, &error))?;
        Ok(CheckReport::from_items(parse_<step>_outdated(output.stdout())))
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn cleanup(&self) -> Result<(), StepError> {
        // Only runs with `--cleanup`; default does nothing. Remove this
        // override when the step has nothing to clean up.
        let mut failures = Vec::new();
        human!("<step>: cleanup");
        if let Err(error) = CommandSpec::new("<tool>")
            .args(["cache", "prune"])
            .neutral_locale()
            .run()
        {
            failures.push(StepError::from_process(ID, Operation::Cleanup, &error));
        }
        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(SnapshotSection::new(
            ID,
            vec![SnapshotCommand::new("<tool>", ["--version"])],
        ))
    }

    fn platforms(&self) -> &'static [Platform] {
        &PLATFORMS
    }

    // Uncomment and override when the step needs them:
    // fn prerequisites(&self) -> &'static [StepId] { &["brew"] }
    // fn conflicts(&self) -> &'static [StepId] { &["mas"] }
    // fn is_user_level(&self) -> bool { true }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `<tool>` is discoverable on `PATH`.
fn detect_<step>() -> Detection {
    if crate::executable::command_exists("<tool>") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses the read-only "what is outdated" output of `<tool>`.
fn parse_<step>_outdated(output: &[u8]) -> Vec<OutdatedItem> {
    // Parse the tool's output here. Return an empty vec when everything is
    // up to date; never fail on output you do not understand.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_has_identity_and_platforms() {
        let step = <Step>;
        assert_eq!(step.id(), ID);
        assert_eq!(step.platforms(), PLATFORMS);
    }
}
```

Notes on the template:

* Steps are `&self` and `Sync`: keep shared mutable state behind
  `Arc<Mutex<_>>` or atomics, never `Rc`/`Cell`. `--check` queries independent
  steps on several threads.
* `detect`, `check` and `snapshot` must be read-only. `run` and `cleanup` are
  the mutating phases.
* `cleanup` runs only with `--cleanup`; the default is a no-op, so delete the
  override when there is nothing to clean. Steps that delete user data need an
  explicit opt-in flag — never clean up by default.
* Do not use `unwrap()`/`expect()` outside tests; map process errors to
  `StepError` with `StepError::run_from_process`/`check_from_process`.
* `human!` prints one progress line (visible when output is not structured);
  see `crates/freshen-lib/src/output.rs`.

## Worked example: a `sample` step

Here is a complete step for a fictional `sample` command-line tool. It detects
`sample` on `PATH`, reports outdated packages with `sample --outdated`, upgrades
with `sample upgrade`, and prunes its cache with `sample cache prune` under
`--cleanup`. Copy the file and replace `sample` with your tool.

`crates/freshen-lib/src/sample.rs`:

```rust
//! The sample step: `sample upgrade`, skipped when `sample` is absent.
//! `sample` manages user-level packages, so it must never run as root.

use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::snapshot::{SnapshotCommand, SnapshotSection};
use crate::step::{
    CheckReport, Detection, Operation, OutdatedItem, Step, StepError, StepId,
};

/// The sample tool step.
pub const SAMPLE: StepId = "sample";

/// sample runs on every platform freshen supports.
const SAMPLE_PLATFORMS: [Platform; 3] =
    [Platform::Macos, Platform::Linux, Platform::Windows];

/// Registers the sample step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(SampleStep)
}

struct SampleStep;

impl SampleStep {
    /// Upgrades every outdated sample package, recording the first failure.
    fn upgrade(&self, failures: &mut Vec<StepError>) {
        human!("sample: upgrade");
        if let Err(error) = CommandSpec::new("sample")
            .args(["upgrade"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
        {
            failures.push(StepError::run_from_process(SAMPLE, &error));
        }
    }
}

impl Step for SampleStep {
    fn id(&self) -> StepId {
        SAMPLE
    }

    fn display_name(&self) -> &str {
        "sample"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_sample())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let output = CommandSpec::new("sample")
            .args(["--outdated"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(SAMPLE, &error))?;
        Ok(CheckReport::from_items(parse_sample_outdated(output.stdout())))
    }

    fn run(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        self.upgrade(&mut failures);
        finish(failures)
    }

    fn cleanup(&self) -> Result<(), StepError> {
        let mut failures = Vec::new();
        human!("sample: cache prune");
        if let Err(error) = CommandSpec::new("sample")
            .args(["cache", "prune"])
            .neutral_locale()
            .run()
        {
            failures.push(StepError::from_process(SAMPLE, Operation::Cleanup, &error));
        }
        finish(failures)
    }

    fn snapshot(&self) -> Option<SnapshotSection> {
        Some(SnapshotSection::new(
            SAMPLE,
            vec![SnapshotCommand::new("sample", ["list", "--versions"])],
        ))
    }

    fn platforms(&self) -> &'static [Platform] {
        &SAMPLE_PLATFORMS
    }

    /// sample manages user-level packages; running it as root installs into
    /// root's environment instead of the user's.
    fn is_user_level(&self) -> bool {
        true
    }
}

fn finish(failures: Vec<StepError>) -> Result<(), StepError> {
    match failures.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn detect_sample() -> Detection {
    if crate::executable::command_exists("sample") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses `sample --outdated`: one package per line, `name current latest`.
fn parse_sample_outdated(output: &[u8]) -> Vec<OutdatedItem> {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let current = parts.next();
            let latest = parts.next();
            let mut item = OutdatedItem::new(name);
            if let Some(current) = current {
                item = item.with_current(current);
            }
            if let Some(latest) = latest {
                item = item.with_latest(latest);
            }
            Some(item)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_display_name() {
        let step = SampleStep;
        assert_eq!(step.id(), SAMPLE);
        assert_eq!(step.display_name(), "sample");
    }

    #[test]
    fn platforms_and_user_level() {
        let step = SampleStep;
        assert_eq!(step.platforms(), &SAMPLE_PLATFORMS);
        assert!(step.is_user_level());
        assert!(!step.needs_root());
        assert!(!step.needs_administrator());
    }

    #[test]
    fn no_prerequisites_or_conflicts_by_default() {
        let step = SampleStep;
        assert!(step.prerequisites().is_empty());
        assert!(step.conflicts().is_empty());
    }

    #[test]
    fn detect_reports_not_installed_when_sample_is_absent() {
        // `sample` is not installed in the test environment, so detection
        // reports NotInstalled and the step is skipped gracefully.
        let step = SampleStep;
        assert!(step
            .detect()
            .is_ok_and(|detection| !detection.is_installed()));
    }

    #[test]
    fn check_reports_outdated_items() {
        let output = b"jq 1.7.1 1.8.0\nripgrep\n";
        let report = parse_sample_outdated(output).into_iter().collect::<CheckReport>();
        assert!(!report.is_up_to_date());
        assert_eq!(report.pending(), 2);

        let mut items = report.items().iter();
        let jq = items.next().unwrap();
        assert_eq!(jq.name(), "jq");
        assert_eq!(jq.current(), Some("1.7.1"));
        assert_eq!(jq.latest(), Some("1.8.0"));

        let ripgrep = items.next().unwrap();
        assert_eq!(ripgrep.name(), "ripgrep");
        assert_eq!(ripgrep.current(), None);
        assert_eq!(ripgrep.latest(), None);
    }

    #[test]
    fn empty_output_is_up_to_date() {
        let report = parse_sample_outdated(b"").into_iter().collect::<CheckReport>();
        assert!(report.is_up_to_date());
        assert_eq!(report.pending(), 0);
    }
}
```

### Registering the example step

To make the step real, add it to the registry:

```rust
// crates/freshen-lib/src/builtin_steps.rs
mod sample;  // not needed: it is already `pub mod` in lib.rs once you add it

let registry = crate::sample::register(registry);
```

in `builtin_steps::register_all`, at the position where the step should run
(prerequisites pull it after their owners anyway — registration order is the
default order, not the dependency order). Then:

1. Add `pub mod sample;` to `crates/freshen-lib/src/lib.rs`.
2. Run `cargo run --example generate_step_docs` to regenerate `docs/steps/`.
3. Add `docs/steps/sample.md` to the `Steps` section of `mkdocs.yml`.
4. Add a test that exercises the step against stub executables on `PATH`
   (see below), then run `cargo test --all-features`.

The docs generator emits `Step id:`, supported platforms, prerequisites and
conflicts straight from the registry; anything beyond that — what the step
detects, checks and upgrades — is written by hand in the module, as above.

## Rules of thumb

* **Graceful skips only.** A missing tool is `Detection::NotInstalled`, never
  an error. A step that cannot run right now returns
  `Detection::Unavailable(reason)` with a `SkipReason` (`NotSignedIn`,
  `NeedsSudo`, `NeedsAdministrator`, `MissingConfiguration`, …).
* **Never quit the terminal that runs freshen**, and restore everything your
  step stopped (apps, containers, VMs) in `restore`, even on error or Ctrl-C.
* **Read-only `check`.** `--check` must not change the machine; it is safe to
  run in CI.
* **Declare your platforms.** `platforms()` is the registry's filter; steps
  that do not list the current platform are never detected, checked or run.
* **No unwrap/expect outside tests.** Map failures to `StepError`; one failing
  step never aborts the run.
* **Cleanup only with `--cleanup`**, and never by default when it deletes user
  data.
* **Object-safe & `Sync`.** Steps are `&self` behind `Box<dyn Step>`; shared
  state uses `Arc<Mutex<_>>` or atomics.

## Testing your step

Steps are tested against stub executables on `PATH`, never against the real
tools. The harness in `crates/freshen/tests/support` (enabled with the
`test-harness` feature) creates stub `brew`, `uv`, `mise`, … executables that
log every invocation and can return canned output, so a step can be replayed
against a recorded command transcript:

```rust
#![cfg(feature = "test-harness")]

mod support;

use support::StubTools;

let tools = StubTools::new(&["sample"]);
tools.set_responses(
    "sample",
    &[
        support::StubOutput::stdout(b"jq 1.7.1 1.8.0\n"),
        support::StubOutput::new(), // `sample upgrade` succeeds silently
    ],
);
// run the binary with `--only sample --check` / `--only sample` and assert
// `tools.step_calls()` contains the expected transcript.
```

See `crates/freshen/tests/tooling.rs` (uv and mise parity tests) for a complete
example. Parser logic deserves its own unit tests with recorded transcripts,
including non-English locales — steps run their commands under a neutral
locale (`CommandSpec::neutral_locale`), but parsers must not crash on
unexpected output.

### Before you open the pull request

* `cargo fmt --all -- --check`
* `cargo clippy --all-targets --all-features -- -D warnings`
* `cargo test --all-features`
* `bash tests/run-tests.sh`
* Regenerate and commit `docs/steps/` plus the `mkdocs.yml` nav entry when a
  step is added, renamed or re-platformed.

Commit with Conventional Commits (`feat:`, `fix:`, `docs:`, …) and reference
the plan item you are implementing.
