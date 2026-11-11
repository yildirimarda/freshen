//! Disk usage reported by `docker system df`.

use std::cell::RefCell;

thread_local! {
    static DOCKER_DF: RefCell<Option<DockerDfUsage>> = const { RefCell::new(None) };
}

/// One row of `docker system df` output: total and active counts, plus the
/// size and reclaimable space reported by the Docker daemon.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DockerDfEntry {
    /// Total number of objects of this type.
    pub total: u64,
    /// Number of objects that are currently in use.
    pub active: u64,
    /// Total size, e.g. `"500MB"`.
    pub size: String,
    /// Space that could be reclaimed, e.g. `"100MB"`.
    pub reclaimable: String,
}

/// Build cache and volume usage reported by `docker system df`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DockerDfUsage {
    pub volumes: Option<DockerDfEntry>,
    pub build_cache: Option<DockerDfEntry>,
}

/// Records the `docker system df` usage produced by the current step.
pub fn record(usage: DockerDfUsage) {
    DOCKER_DF.with(|usage_cell| *usage_cell.borrow_mut() = Some(usage));
}

/// Takes the `docker system df` usage recorded on the current thread.
pub fn take() -> DockerDfUsage {
    DOCKER_DF
        .with(|usage_cell| usage_cell.borrow_mut().take())
        .unwrap_or_default()
}

/// Parses `docker system df` output and returns the volume and build-cache rows.
///
/// Rows that do not match `Local Volumes` or `Build Cache` are ignored.
#[must_use]
pub fn parse(output: &str) -> DockerDfUsage {
    let mut usage = DockerDfUsage::default();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("Local") if fields.next() == Some("Volumes") => {
                usage.volumes = parse_entry(fields);
            }
            Some("Build") if fields.next() == Some("Cache") => {
                usage.build_cache = parse_entry(fields);
            }
            _ => {}
        }
    }
    usage
}

fn parse_entry(mut fields: std::str::SplitWhitespace<'_>) -> Option<DockerDfEntry> {
    let total = fields.next()?.parse().ok()?;
    let active = fields.next()?.parse().ok()?;
    let size = fields.next()?.to_owned();
    let reclaimable = fields.next()?.to_owned();
    Some(DockerDfEntry {
        total,
        active,
        size,
        reclaimable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_volumes_and_build_cache() {
        let output = "\
TYPE            TOTAL     ACTIVE    SIZE      RECLAIMABLE
Images          2         2         1.2GB     0B
Containers      1         1         50MB      0B
Local Volumes   3         2         500MB     100MB
Build Cache     10        2         2.5GB     1.5GB
";

        let usage = parse(output);

        assert_eq!(
            usage.volumes,
            Some(DockerDfEntry {
                total: 3,
                active: 2,
                size: "500MB".to_owned(),
                reclaimable: "100MB".to_owned(),
            })
        );
        assert_eq!(
            usage.build_cache,
            Some(DockerDfEntry {
                total: 10,
                active: 2,
                size: "2.5GB".to_owned(),
                reclaimable: "1.5GB".to_owned(),
            })
        );
    }

    #[test]
    fn ignores_rows_it_does_not_recognise() {
        let output = "\
TYPE            TOTAL     ACTIVE    SIZE      RECLAIMABLE
Images          2         2         1.2GB     0B
Containers      1         1         50MB      0B
";

        let usage = parse(output);
        assert!(usage.volumes.is_none());
        assert!(usage.build_cache.is_none());
    }

    #[test]
    fn empty_output_yields_no_usage() {
        let usage = parse("");
        assert!(usage.volumes.is_none());
        assert!(usage.build_cache.is_none());
    }

    #[test]
    fn recording_outside_a_recorder_is_harmless() {
        let usage = DockerDfUsage::default();
        record(usage.clone());
        assert_eq!(take(), usage);
        assert_eq!(take(), DockerDfUsage::default());
    }
}
