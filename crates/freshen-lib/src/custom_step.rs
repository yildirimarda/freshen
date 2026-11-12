//! User-defined steps loaded from the configuration file.
//!
//! Anything freshen does not have a built-in step for can be added without
//! changing the code by listing a command under `[[custom_steps]]`:
//!
//! ```toml
//! [[custom_steps]]
//! name = "my-linter"
//! command = ["golangci-lint", "run"]
//! platforms = ["linux", "macos"]
//! order = 5
//! ```
//!
//! `name` is the step id and the name shown to the user. `command` is the
//! program and its arguments. `platforms` restricts the step to the named
//! operating systems (every platform when absent). `order` places the step in
//! the run sequence: built-in steps keep their registration order and custom
//! steps sort by `order`, so a custom step with an order smaller than a
//! built-in step's position runs before it and a larger order runs after it.
//! Custom steps that share an order keep their configuration order.

use serde::Deserialize;

use crate::executable;
use crate::platform::Platform;
use crate::process::CommandSpec;
use crate::step::{CheckReport, Detection, Step, StepError, StepId};

/// One custom step as written in the configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomStepConfig {
    /// The step id and the name shown to the user.
    pub name: String,
    /// The program and its arguments.
    pub command: Vec<String>,
    /// The platforms the step runs on; every platform when absent.
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    /// The position in the run sequence; runs after the built-in steps when
    /// absent.
    #[serde(default)]
    pub order: Option<usize>,
}

/// A problem with a user-defined custom step.
#[derive(Debug, thiserror::Error)]
pub enum CustomStepError {
    /// The step has no name.
    #[error("custom step has an empty name")]
    EmptyName,

    /// The step's command lists no program to run.
    #[error("custom step `{name}` has an empty command")]
    EmptyCommand {
        /// The name of the step with the empty command.
        name: String,
    },

    /// The step names a platform freshen does not know.
    #[error("custom step `{name}` names unknown platform `{platform}`")]
    UnknownPlatform {
        /// The name of the step that named the platform.
        name: String,
        /// The platform name that is not known.
        platform: String,
    },
}

/// A user-defined step that runs a command.
pub struct CustomStep {
    /// The step id, leaked so it lives for the whole process.
    id: StepId,
    /// The name shown to the user.
    name: &'static str,
    /// The program and its arguments.
    command: CommandSpec,
    /// The platforms the step runs on.
    platforms: &'static [Platform],
    /// The position in the run sequence.
    order: usize,
}

impl CustomStep {
    /// Validates one configuration entry and turns it into a step.
    ///
    /// Strings and the platform list are leaked, so the step lives for the
    /// whole process and the `Step` trait's `&'static` references can point at
    /// them.
    ///
    /// # Errors
    ///
    /// Returns [`CustomStepError`] when the name or command is empty or a
    /// named platform is unknown.
    pub fn try_from_config(config: &CustomStepConfig) -> Result<Self, CustomStepError> {
        if config.name.is_empty() {
            return Err(CustomStepError::EmptyName);
        }
        if config.command.is_empty() {
            return Err(CustomStepError::EmptyCommand {
                name: config.name.clone(),
            });
        }

        let platforms = match &config.platforms {
            Some(names) => {
                let mut out = Vec::with_capacity(names.len());
                for name in names {
                    let Some(platform) = Platform::from_name(name) else {
                        return Err(CustomStepError::UnknownPlatform {
                            name: config.name.clone(),
                            platform: name.clone(),
                        });
                    };
                    out.push(platform);
                }
                out
            }
            None => Platform::ALL.to_vec(),
        };

        let id = Box::leak(config.name.clone().into_boxed_str());
        let platforms = Box::leak(platforms.into_boxed_slice());
        let mut command = CommandSpec::new(config.command[0].clone());
        command = command.args(config.command[1..].iter().cloned());

        Ok(Self {
            id,
            name: id,
            command,
            platforms,
            order: config.order.unwrap_or(usize::MAX),
        })
    }

    /// The position of the step in the run sequence.
    #[must_use]
    pub fn order(&self) -> usize {
        self.order
    }
}

impl Step for CustomStep {
    fn id(&self) -> StepId {
        self.id
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn detect(&self) -> Result<Detection, StepError> {
        let program = self.command.program().to_string_lossy();
        if executable::command_exists(program.as_ref()) {
            Ok(Detection::Installed)
        } else {
            Ok(Detection::NotInstalled)
        }
    }

    fn check(&self) -> Result<CheckReport, StepError> {
        Ok(CheckReport::up_to_date())
    }

    fn run(&self) -> Result<(), StepError> {
        self.command
            .run()
            .map(|_| ())
            .map_err(|error| StepError::run_from_process(self.id, &error))
    }

    fn platforms(&self) -> &'static [Platform] {
        self.platforms
    }
}

#[cfg(test)]
mod tests {
    use super::{CustomStep, CustomStepConfig, CustomStepError};
    use crate::platform::Platform;
    use crate::step::{Detection, Step};
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config(name: &str, command: &[&str]) -> CustomStepConfig {
        CustomStepConfig {
            name: name.to_owned(),
            command: command.iter().map(|command| command.to_string()).collect(),
            platforms: None,
            order: None,
        }
    }

    fn parse(config: &CustomStepConfig) -> CustomStep {
        match CustomStep::try_from_config(config) {
            Ok(step) => step,
            Err(error) => panic!("expected a valid custom step, got {error}"),
        }
    }

    fn invalid(config: &CustomStepConfig) -> CustomStepError {
        match CustomStep::try_from_config(config) {
            Ok(_) => panic!("expected an invalid custom step"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_valid_custom_step_keeps_its_name_command_and_order() {
        let step = parse(&config("my-linter", &["golangci-lint", "run"]));

        assert_eq!(step.id(), "my-linter");
        assert_eq!(step.display_name(), "my-linter");
        assert_eq!(step.order(), usize::MAX);
        assert_eq!(step.platforms(), Platform::ALL);
    }

    #[test]
    fn command_and_order_are_read_from_the_configuration() {
        let mut config = config("my-linter", &["golangci-lint", "run"]);
        config.platforms = Some(vec!["linux".to_owned(), "macos".to_owned()]);
        config.order = Some(3);

        let step = parse(&config);

        assert_eq!(step.platforms(), [Platform::Linux, Platform::Macos]);
        assert_eq!(step.order(), 3);
    }

    #[test]
    fn every_platform_is_the_default_when_platforms_is_absent() {
        let step = parse(&config("any", &["any"]));

        assert_eq!(step.platforms(), Platform::ALL);
    }

    #[test]
    fn an_empty_name_is_rejected() {
        let error = invalid(&config("", &["echo"]));

        assert!(matches!(error, CustomStepError::EmptyName));
    }

    #[test]
    fn an_empty_command_is_rejected() {
        let error = invalid(&config("nope", &[]));

        let CustomStepError::EmptyCommand { name } = error else {
            panic!("expected an empty command error, got {error:?}");
        };
        assert_eq!(name, "nope");
    }

    #[test]
    fn an_unknown_platform_is_rejected() {
        let mut config = config("bad", &["echo"]);
        config.platforms = Some(vec!["freebsd".to_owned()]);

        let error = invalid(&config);

        let CustomStepError::UnknownPlatform { name, platform } = error else {
            panic!("expected an unknown platform error, got {error:?}");
        };
        assert_eq!(name, "bad");
        assert_eq!(platform, "freebsd");
    }

    fn temp_executable() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            env::temp_dir().join(format!("freshen-custom-step-{}-{next}", std::process::id()));
        fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("custom-tool");
            fs::write(&path, "#!/bin/sh\nexit 0\n")
                .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap_or_else(|error| {
                panic!("failed to make {} executable: {error}", path.display())
            });
            path
        }
        #[cfg(windows)]
        {
            let path = dir.join("custom-tool.bat");
            fs::write(&path, "@echo off\r\nexit /b 0\r\n")
                .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
            path
        }
    }

    #[test]
    fn detect_reports_installed_when_the_program_exists() {
        let path = temp_executable();
        let step = parse(&config("tool", &[path.to_string_lossy().as_ref()]));

        assert!(step.detect().is_ok_and(Detection::is_installed));

        let _ = fs::remove_dir_all(path.parent().unwrap_or(Path::new(".")));
    }

    #[test]
    fn detect_reports_not_installed_when_the_program_is_absent() {
        let step = parse(&config("tool", &["freshen-does-not-exist-xyz"]));

        assert!(step
            .detect()
            .is_ok_and(|detection| !detection.is_installed()));
    }

    #[test]
    fn check_is_read_only_and_up_to_date() {
        let step = parse(&config("any", &["echo"]));

        let report = step
            .check()
            .unwrap_or_else(|error| panic!("check failed: {error}"));
        assert!(report.is_up_to_date());
    }

    #[test]
    fn run_executes_the_command() {
        let path = temp_executable();
        let step = parse(&config("tool", &[path.to_string_lossy().as_ref()]));

        assert!(step.run().is_ok());

        let _ = fs::remove_dir_all(path.parent().unwrap_or(Path::new(".")));
    }

    #[test]
    fn steps_are_object_safe_and_dispatch_through_the_trait() {
        let step: Box<dyn Step> = Box::new(parse(&config("any", &["echo"])));

        assert_eq!(step.id(), "any");
        assert_eq!(step.display_name(), "any");
        assert_eq!(step.platforms(), Platform::ALL);
    }
}
