//! Validation tests for the Scoop bucket manifest.
//!
//! The manifest lives at `bucket/freshen.json` and is the canonical copy that
//! is published to the `yildirimarda/scoop-bucket` repository so Windows
//! users can install freshen with `scoop install freshen`. These tests prove
//! the manifest is well-formed and points at the release assets built by
//! `ci-proposals/release-binaries.yml`. They read only a static file, so they
//! run on every platform.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The canonical Scoop bucket manifest, relative to the repository root.
const MANIFEST_RELATIVE_PATH: &str = "bucket/freshen.json";

/// The Windows x86_64 release asset the manifest must reference.
const EXPECTED_URL: &str =
    "https://github.com/yildirimarda/freshen/releases/download/v0.1.0/freshen-0.1.0-x86_64-pc-windows-msvc.zip";

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(MANIFEST_RELATIVE_PATH)
}

fn manifest() -> Value {
    let path = manifest_path();
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()))
}

#[test]
fn manifest_is_a_well_formed_scoop_manifest() {
    let manifest = manifest();
    assert!(manifest.is_object(), "manifest must be a JSON object");

    for field in [
        "version",
        "architecture",
        "homepage",
        "license",
        "description",
    ] {
        assert!(
            manifest.get(field).is_some(),
            "manifest is missing the `{field}` field"
        );
    }
}

#[test]
fn manifest_publishes_version_zero_one_zero() {
    assert_eq!(manifest()["version"], "0.1.0");
}

#[test]
fn manifest_references_the_windows_x86_64_release_asset() {
    let manifest = manifest();
    let architecture = &manifest["architecture"];
    assert!(architecture.is_object(), "architecture must be an object");
    assert!(
        architecture.get("64bit").is_some(),
        "architecture must define a 64bit build"
    );
    let build = &architecture["64bit"];
    assert_eq!(build["url"], EXPECTED_URL);
    assert_eq!(build["bin"], "freshen.exe");
}

#[test]
fn manifest_hash_is_a_sha256_placeholder() {
    let manifest = manifest();
    let hash = &manifest["architecture"]["64bit"]["hash"];
    let hash = hash
        .as_str()
        .unwrap_or_else(|| panic!("hash must be a string"));
    assert!(
        hash.starts_with("sha256:"),
        "hash must use the sha256 prefix: {hash}"
    );
    let digest = &hash["sha256:".len()..];
    assert_eq!(
        digest.len(),
        64,
        "sha256 digest must be 64 hex digits: {digest}"
    );
    assert!(
        digest.chars().all(|c| c.is_ascii_hexdigit()),
        "sha256 digest must be hexadecimal: {digest}"
    );
}

#[test]
fn manifest_uses_the_mit_license() {
    assert_eq!(manifest()["license"], "MIT");
}

#[test]
fn manifest_names_the_project_homepage() {
    assert_eq!(
        manifest()["homepage"],
        "https://github.com/yildirimarda/freshen"
    );
}

#[test]
fn manifest_describes_the_project() {
    let manifest = manifest();
    let description = manifest["description"]
        .as_str()
        .unwrap_or_else(|| panic!("description must be a string"));
    assert!(!description.is_empty());
}
