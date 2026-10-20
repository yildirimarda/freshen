# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- Managed by release-please from Conventional Commits. Do not edit by hand. -->

## [Unreleased]

### Changed

- The Rust `freshen` binary is now the implementation; the zsh script has been retired.

### Removed

- The zsh implementation of the update steps. Use `freshen`; see the migration section in the README.
- The deprecated `update-all.sh` compatibility shim.
