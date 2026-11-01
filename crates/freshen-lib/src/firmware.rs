//! The firmware updates step: `fwupdmgr get-updates` reports what is
//! available and `fwupdmgr update` installs them when explicitly requested.
//!
//! `fwupdmgr` is the command line tool of fwupd, the Linux firmware update
//! daemon. It lists the firmware updates that are available
//! (`fwupdmgr get-updates`) and installs them (`fwupdmgr update`). Only the
//! list works without administrator rights, so the step can always report what
//! is available.
//!
//! Installing a firmware update can require administrator rights and a reboot,
//! so the step only reports the available updates, unless
//! `--install-firmware-updates` asks for the installation.
//!
//! The step only runs on Linux, because `fwupdmgr` exists only there.

use crate::options::RunOptions;
use crate::output::human;
use crate::platform::Platform;
use crate::process::{CommandSpec, NETWORK_RETRIES};
use crate::registry::Registry;
use crate::step::{CheckReport, Detection, OutdatedItem, Step, StepError, StepId};

/// The firmware updates step.
pub const FIRMWARE: StepId = "firmware";

/// fwupdmgr only runs on Linux.
const FIRMWARE_PLATFORMS: [Platform; 1] = [Platform::Linux];

/// Registers the firmware updates step.
#[must_use]
pub fn register(registry: Registry) -> Registry {
    registry.with(FirmwareStep::new())
}

struct FirmwareStep {
    /// The output of the last `fwupdmgr get-updates`, so that a run which only
    /// reports the updates does not ask the firmware daemon a second time.
    last_list: std::sync::Mutex<Option<String>>,
}

impl FirmwareStep {
    fn new() -> Self {
        Self {
            last_list: std::sync::Mutex::new(None),
        }
    }

    /// Lists the available firmware updates without changing anything.
    fn get_updates(&self) -> Result<String, StepError> {
        let output = CommandSpec::new("fwupdmgr")
            .args(["get-updates"])
            .neutral_locale()
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map_err(|error| StepError::check_from_process(FIRMWARE, &error))?;
        Ok(output.stdout().to_owned())
    }

    /// Installs the available firmware updates.
    fn install(&self) -> Result<(), StepError> {
        human!("firmware: install (firmware updates)");
        CommandSpec::new("fwupdmgr")
            .args(["update"])
            .retry_transient_network(NETWORK_RETRIES)
            .run()
            .map(|_| ())
            .map_err(|error| StepError::run_from_process(FIRMWARE, &error))
    }

    /// The listing of the pending updates, reused from the check of the run.
    fn listing(&self) -> Result<String, StepError> {
        let cached = self.last_list.lock().ok().and_then(|last| last.clone());
        match cached {
            Some(text) => Ok(text),
            None => self.get_updates(),
        }
    }

    /// Reports the available updates without installing them.
    fn report(&self) -> Result<(), StepError> {
        let text = self.listing()?;
        let items = parse_get_updates(&text);
        if let Ok(mut last) = self.last_list.lock() {
            *last = Some(text.clone());
        }
        if items.is_empty() {
            human!("firmware: no firmware updates available");
        } else {
            human!(
                "firmware: {} update(s) available; nothing installed (use --install-firmware-updates)",
                items.len()
            );
            for item in &items {
                human!("  - {item}");
            }
        }
        Ok(())
    }
}

impl Default for FirmwareStep {
    fn default() -> Self {
        Self::new()
    }
}

impl Step for FirmwareStep {
    fn id(&self) -> StepId {
        FIRMWARE
    }

    fn display_name(&self) -> &str {
        "firmware updates"
    }

    fn detect(&self) -> Result<Detection, StepError> {
        Ok(detect_firmware())
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        let text = self.get_updates()?;
        let items = parse_get_updates(&text);
        if let Ok(mut last) = self.last_list.lock() {
            *last = Some(text.clone());
        }
        Ok(CheckReport::from_items(items))
    }

    fn run(&self) -> Result<(), StepError> {
        self.run_with_options(&RunOptions::new())
    }

    fn run_with_options(&self, options: &RunOptions) -> Result<(), StepError> {
        if options.install_firmware_updates() {
            self.install()
        } else {
            self.report()
        }
    }

    fn platforms(&self) -> &'static [Platform] {
        &FIRMWARE_PLATFORMS
    }
}

/// Whether the `fwupdmgr` binary is discoverable on `PATH`.
fn detect_firmware() -> Detection {
    if crate::executable::command_exists("fwupdmgr") {
        Detection::Installed
    } else {
        Detection::NotInstalled
    }
}

/// Parses the output of `fwupdmgr get-updates`.
///
/// Each device that has an update available is a block separated by a blank
/// line. The first line of the block is the device id (terminated by `:`), the
/// rest are `Key: Value` fields. `Version:` appears twice when an update is
/// available: the installed version first and the available version second.
/// Devices that are already up to date (a single version, or two equal ones)
/// and the "No updates found." message are ignored.
fn parse_get_updates(output: &str) -> Vec<OutdatedItem> {
    output
        .split("\n\n")
        .filter_map(parse_device_block)
        .collect()
}

fn parse_device_block(block: &str) -> Option<OutdatedItem> {
    let mut lines = block.lines();
    let header = lines.next()?;
    let device_id = header.strip_suffix(':')?.trim();
    if device_id.is_empty() {
        return None;
    }

    let mut name = device_id.to_owned();
    let mut versions: Vec<&str> = Vec::new();
    for line in lines {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Device" if !value.trim().is_empty() => name = value.trim().to_owned(),
            "Version" => versions.push(value.trim()),
            _ => {}
        }
    }

    let current = versions.first()?;
    let latest = versions.last()?;
    if current == latest {
        return None;
    }

    let mut item = OutdatedItem::new(name);
    item = item.with_current(*current).with_latest(*latest);
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::{parse_device_block, parse_get_updates};

    /// The output of `fwupdmgr get-updates` for one device with an update.
    const ONE_UPDATE: &str = "1234abcd-1234-1234-1234-1234abcd1234:\n\
         \tDevice: ThinkPad Thunderbolt Controller\n\
         \tSummary: Thunderbolt firmware update\n\
         \tVersion: 1.2.3\n\
         \tVersion: 1.2.4\n\
         \tRemote id: lvfs\n";

    #[test]
    fn parses_a_device_with_current_and_available_version() {
        let items = parse_get_updates(ONE_UPDATE);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "ThinkPad Thunderbolt Controller");
        assert_eq!(items[0].current(), Some("1.2.3"));
        assert_eq!(items[0].latest(), Some("1.2.4"));
    }

    #[test]
    fn uses_the_device_id_when_there_is_no_device_field() {
        let items = parse_get_updates(
            "1234abcd-1234-1234-1234-1234abcd1234:\n\
             \tVersion: 1.2.3\n\
             \tVersion: 1.2.4\n",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name(), "1234abcd-1234-1234-1234-1234abcd1234");
        assert_eq!(items[0].current(), Some("1.2.3"));
        assert_eq!(items[0].latest(), Some("1.2.4"));
    }

    #[test]
    fn ignores_a_device_that_is_already_up_to_date() {
        let items = parse_get_updates(
            "1234abcd-1234-1234-1234-1234abcd1234:\n\
             \tDevice: Up to Date\n\
             \tVersion: 1.2.3\n\
             \tVersion: 1.2.3\n",
        );

        assert!(items.is_empty());
    }

    #[test]
    fn ignores_a_device_with_only_one_version() {
        let items = parse_get_updates(
            "1234abcd-1234-1234-1234-1234abcd1234:\n\
             \tDevice: Only One\n\
             \tVersion: 1.2.3\n",
        );

        assert!(items.is_empty());
    }

    #[test]
    fn no_updates_found_means_no_items() {
        assert!(parse_get_updates("No updates found.\n").is_empty());
        assert!(parse_get_updates("").is_empty());
        assert!(parse_get_updates("   \n").is_empty());
    }

    #[test]
    fn parses_multiple_devices() {
        let items = parse_get_updates(
            "1234abcd-1234-1234-1234-1234abcd1234:\n\
             \tDevice: Device A\n\
             \tVersion: 1.0\n\
             \tVersion: 1.1\n\
\n\
             5678efgh-5678-5678-5678-5678efgh5678:\n\
             \tDevice: Device B\n\
             \tVersion: 2.0\n\
             \tVersion: 2.1\n",
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name(), "Device A");
        assert_eq!(items[0].current(), Some("1.0"));
        assert_eq!(items[0].latest(), Some("1.1"));
        assert_eq!(items[1].name(), "Device B");
        assert_eq!(items[1].current(), Some("2.0"));
        assert_eq!(items[1].latest(), Some("2.1"));
    }

    #[test]
    fn a_block_without_a_device_id_header_is_ignored() {
        assert!(parse_device_block("No updates found.").is_none());
        assert!(parse_device_block("Device: Something\n\tVersion: 1.0").is_none());
    }
}
