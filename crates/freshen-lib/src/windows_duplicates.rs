//! Reporting Windows packages that are installed by more than one package
//! manager (winget, Chocolatey, Scoop) so the same application is not
//! upgraded twice unnoticed.
//!
//! This is a read-only, report-only check: it inspects installed package
//! lists and reports the packages that appear under more than one manager,
//! tagged with the managers that installed them. It never changes the
//! machine. The package lists are injected, so the check is unit-tested on
//! every operating system without touching a real package manager.

use std::collections::{BTreeMap, BTreeSet};

/// A package installed by more than one Windows package manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicatePackage {
    /// Name of the package, as reported by the first manager that listed it.
    pub name: String,
    /// Ids of the package managers that have the package installed, sorted and
    /// deduplicated.
    pub managers: Vec<String>,
}

/// Reports the packages that appear in more than one installed package list.
///
/// `lists` pairs each package manager id with the names of the packages it
/// has installed. Packages that appear under more than one manager are
/// returned, each tagged with the sorted, deduplicated ids of the managers
/// that installed them. The result is sorted by package name for stable
/// output.
///
/// This is read-only: it inspects the lists and reports duplicates without
/// changing anything, so it is safe as a report-only check.
#[must_use]
pub fn find_duplicates(lists: &[(&str, &[String])]) -> Vec<DuplicatePackage> {
    let mut by_package: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for (manager, packages) in lists {
        for name in packages.iter() {
            by_package.entry(name.clone()).or_default().insert(manager);
        }
    }

    by_package
        .into_iter()
        .filter_map(|(name, managers)| {
            if managers.len() > 1 {
                Some(DuplicatePackage {
                    name,
                    managers: managers.into_iter().map(ToString::to_string).collect(),
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINGET: &str = "winget";
    const CHOCOLATEY: &str = "chocolatey";
    const SCOOP: &str = "scoop";

    #[test]
    fn reports_no_duplicates_when_each_package_has_one_manager() {
        let duplicates = find_duplicates(&[
            (WINGET, &["git".to_owned(), "7zip".to_owned()]),
            (CHOCOLATEY, &["python".to_owned(), "node".to_owned()]),
            (SCOOP, &["curl".to_owned()]),
        ]);

        assert!(duplicates.is_empty());
    }

    #[test]
    fn reports_a_package_installed_by_two_managers() {
        let duplicates = find_duplicates(&[
            (WINGET, &["git".to_owned(), "7zip".to_owned()]),
            (CHOCOLATEY, &["git".to_owned(), "python".to_owned()]),
            (SCOOP, &["curl".to_owned()]),
        ]);

        assert_eq!(
            duplicates,
            [DuplicatePackage {
                name: "git".to_owned(),
                managers: vec![CHOCOLATEY.to_owned(), WINGET.to_owned()],
            }]
        );
    }

    #[test]
    fn reports_a_package_installed_by_all_three_managers() {
        let duplicates = find_duplicates(&[
            (WINGET, &["git".to_owned()]),
            (CHOCOLATEY, &["git".to_owned()]),
            (SCOOP, &["git".to_owned()]),
        ]);

        assert_eq!(
            duplicates,
            [DuplicatePackage {
                name: "git".to_owned(),
                managers: vec![CHOCOLATEY.to_owned(), SCOOP.to_owned(), WINGET.to_owned()],
            }]
        );
    }

    #[test]
    fn dedupes_a_package_listed_twice_by_the_same_manager() {
        let duplicates = find_duplicates(&[
            (WINGET, &["git".to_owned(), "git".to_owned()]),
            (CHOCOLATEY, &["git".to_owned()]),
        ]);

        assert_eq!(
            duplicates,
            [DuplicatePackage {
                name: "git".to_owned(),
                managers: vec![CHOCOLATEY.to_owned(), WINGET.to_owned()],
            }]
        );
    }

    #[test]
    fn reports_multiple_duplicates_sorted_by_name() {
        let duplicates = find_duplicates(&[
            (
                WINGET,
                &["git".to_owned(), "7zip".to_owned(), "python".to_owned()],
            ),
            (CHOCOLATEY, &["python".to_owned(), "node".to_owned()]),
            (SCOOP, &["7zip".to_owned(), "git".to_owned()]),
        ]);

        assert_eq!(
            duplicates,
            [
                DuplicatePackage {
                    name: "7zip".to_owned(),
                    managers: vec![SCOOP.to_owned(), WINGET.to_owned()],
                },
                DuplicatePackage {
                    name: "git".to_owned(),
                    managers: vec![SCOOP.to_owned(), WINGET.to_owned()],
                },
                DuplicatePackage {
                    name: "python".to_owned(),
                    managers: vec![CHOCOLATEY.to_owned(), WINGET.to_owned()],
                },
            ]
        );
    }

    #[test]
    fn empty_lists_report_nothing() {
        assert!(find_duplicates(&[]).is_empty());
        assert!(find_duplicates(&[(WINGET, &[]), (CHOCOLATEY, &[])]).is_empty());
    }

    #[test]
    fn case_sensitive_matching_treats_differently_cased_names_as_distinct() {
        let duplicates = find_duplicates(&[
            (WINGET, &["Git".to_owned()]),
            (CHOCOLATEY, &["git".to_owned()]),
        ]);

        assert!(duplicates.is_empty());
    }
}
