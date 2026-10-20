//! Operating systems a step can target.

use std::fmt;

/// An operating system freshen supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// macOS.
    Macos,
    /// Linux distributions.
    Linux,
    /// Windows.
    Windows,
}

impl Platform {
    /// Every platform freshen supports.
    pub const ALL: [Self; 3] = [Self::Macos, Self::Linux, Self::Windows];

    /// The platform this binary was compiled for, if freshen supports it.
    #[must_use]
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else {
            None
        }
    }

    /// Lowercase name used in configuration, CLI arguments and logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Platform;

    #[test]
    fn names_are_lowercase() {
        assert_eq!(Platform::Macos.as_str(), "macos");
        assert_eq!(Platform::Linux.as_str(), "linux");
        assert_eq!(Platform::Windows.as_str(), "windows");
    }

    #[test]
    fn display_matches_as_str() {
        for platform in Platform::ALL {
            assert_eq!(platform.to_string(), platform.as_str());
        }
    }

    #[test]
    fn current_matches_the_target_os() {
        let expected = if cfg!(target_os = "macos") {
            Some(Platform::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Platform::Linux)
        } else if cfg!(target_os = "windows") {
            Some(Platform::Windows)
        } else {
            None
        };

        assert_eq!(Platform::current(), expected);
    }

    #[test]
    fn all_contains_every_platform_once() {
        assert_eq!(Platform::ALL.len(), 3);
        for platform in Platform::ALL {
            assert_eq!(
                Platform::ALL
                    .iter()
                    .filter(|candidate| **candidate == platform)
                    .count(),
                1
            );
        }
    }
}
