//! Fuzz targets for freshen's parsers.
//!
//! Each target feeds arbitrary bytes into a pure string parser so that
//! malformed or unexpected command output never crashes a freshen run.
//! Targets live in `fuzz_targets/`; this crate is only the harness.
