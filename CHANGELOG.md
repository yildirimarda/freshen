# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- Managed by release-please from Conventional Commits. Do not edit by hand. -->

## 0.1.0 (2026-10-21)


### Features

* add --list-steps with detection and skip reasons ([#13](https://github.com/yildirimarda/freshen/issues/13)) ([ff64294](https://github.com/yildirimarda/freshen/commit/ff6429462e674e063565bb928226c851ea95f634))
* add an MSRV check job to the Rust CI proposal ([#20](https://github.com/yildirimarda/freshen/issues/20)) ([4c0fdca](https://github.com/yildirimarda/freshen/commit/4c0fdca9b7aca10e1095ea5d34bf8d5ca60bec29))
* add cross-platform run flags and dry-run planning ([#10](https://github.com/yildirimarda/freshen/issues/10)) ([ab319a0](https://github.com/yildirimarda/freshen/commit/ab319a0511a4be687b6c43947764ae30af0cadc9))
* add per-step and global timeouts with a timed out status ([#40](https://github.com/yildirimarda/freshen/issues/40)) ([4b24790](https://github.com/yildirimarda/freshen/commit/4b2479000b621cc57c7204587bfa8f53564d997a))
* add platform-aware ordered step registry ([#6](https://github.com/yildirimarda/freshen/issues/6)) ([63b89e0](https://github.com/yildirimarda/freshen/commit/63b89e0710b2a1a63b5b33dd673e6d59e0aff3c8))
* add preflight checks before updates ([#35](https://github.com/yildirimarda/freshen/issues/35)) ([7164df7](https://github.com/yildirimarda/freshen/commit/7164df7834ac8997068d95e6072e2be85532ec92))
* assign Windows child to its job before resuming ([#56](https://github.com/yildirimarda/freshen/issues/56)) ([805198e](https://github.com/yildirimarda/freshen/commit/805198e315fe266fd1aefcf46c28bf997b82e684))
* define the step abstraction ([#5](https://github.com/yildirimarda/freshen/issues/5)) ([8851507](https://github.com/yildirimarda/freshen/commit/88515072a00a2ffa6f98bbe5c100c35a53f36087))
* detect host OS, architecture, privilege and terminal ([#8](https://github.com/yildirimarda/freshen/issues/8)) ([c64d07b](https://github.com/yildirimarda/freshen/commit/c64d07bc6b3d72025002d3a230ea9eadc4fb32a1))
* detect WSL and container environments ([#24](https://github.com/yildirimarda/freshen/issues/24)) ([935a645](https://github.com/yildirimarda/freshen/commit/935a64581b231b2d9906a13ac8fe2b930300fed0))
* document and stabilize process exit codes ([#45](https://github.com/yildirimarda/freshen/issues/45)) ([6abfdcb](https://github.com/yildirimarda/freshen/commit/6abfdcb31eee977dd51398ddb41c8ca20b51fe9e))
* gate destructive cleanup behind --cleanup and log reclaimed space ([#44](https://github.com/yildirimarda/freshen/issues/44)) ([7008db8](https://github.com/yildirimarda/freshen/commit/7008db82800e82b6f77575a13158ad1f911cdd05))
* kill the whole process tree on timeout or interrupt ([#46](https://github.com/yildirimarda/freshen/issues/46)) ([8b3308a](https://github.com/yildirimarda/freshen/commit/8b3308a7ddc34b29a2bf98661aa0e0c6fdc8d63f))
* make a repeated run a no-op ([#39](https://github.com/yildirimarda/freshen/issues/39)) ([d369780](https://github.com/yildirimarda/freshen/commit/d369780dc6ff1ebfb7a53985c7ca0a582c86c8b6))
* port app quit/reopen logic with parity tests ([#15](https://github.com/yildirimarda/freshen/issues/15)) ([bb1e239](https://github.com/yildirimarda/freshen/commit/bb1e2394db2f93fdcf2edda9ed6cc4c0003f582a))
* port Colima/lima/Docker orchestration with parity tests ([#16](https://github.com/yildirimarda/freshen/issues/16)) ([31ddc0b](https://github.com/yildirimarda/freshen/commit/31ddc0bb8808bdf1d6345aceb49cdd0347492998))
* port Homebrew steps with cask artifact detection ([#14](https://github.com/yildirimarda/freshen/issues/14)) ([3cce13e](https://github.com/yildirimarda/freshen/commit/3cce13e6923e535e06683f47a71240bc78cec305))
* port uv and mise steps with parity tests ([#17](https://github.com/yildirimarda/freshen/issues/17)) ([21cba1a](https://github.com/yildirimarda/freshen/commit/21cba1aec9e81b6200b7608c036858ed9a9f75d1))
* print rollback hints from the pre-upgrade snapshot ([#34](https://github.com/yildirimarda/freshen/issues/34)) ([203cace](https://github.com/yildirimarda/freshen/commit/203cace516b3d02e5ae435c407b5e6cddeaffb35))
* probe Windows process table for stale toolchains ([#53](https://github.com/yildirimarda/freshen/issues/53)) ([09a0626](https://github.com/yildirimarda/freshen/commit/09a06268c25d7b45f20b823b32af510b4362eedc))
* prune old pre-upgrade snapshots with configurable retention ([#47](https://github.com/yildirimarda/freshen/issues/47)) ([37deb3f](https://github.com/yildirimarda/freshen/commit/37deb3f192e86b8b50f2fa38b12f556a6f0b60c5))
* record pre-upgrade version snapshots ([#33](https://github.com/yildirimarda/freshen/issues/33)) ([c09dcf7](https://github.com/yildirimarda/freshen/commit/c09dcf7552b868fe1c7a72460d8403f78193fbf2))
* report specific network preflight failures ([#60](https://github.com/yildirimarda/freshen/issues/60)) ([307b5ec](https://github.com/yildirimarda/freshen/commit/307b5ecde02409e0a2b25186b3d60bfa67f33fb3))
* retire the zsh implementation and document the migration ([#19](https://github.com/yildirimarda/freshen/issues/19)) ([af0b152](https://github.com/yildirimarda/freshen/commit/af0b152dc1654b0f4f456c31a214f9678e997865))
* retry transient network failures and skip unreachable registries ([#41](https://github.com/yildirimarda/freshen/issues/41)) ([c10231a](https://github.com/yildirimarda/freshen/commit/c10231a32c174e24232b903c10f2267df7e76da4))
* scaffold cargo workspace with pinned toolchain, MSRV and rust CI gates ([#2](https://github.com/yildirimarda/freshen/issues/2)) ([87a5df0](https://github.com/yildirimarda/freshen/commit/87a5df05bb1df639121c467785dbf3b3eb622d26))
* show conflict skips in --list-steps ([#30](https://github.com/yildirimarda/freshen/issues/30)) ([8890c22](https://github.com/yildirimarda/freshen/commit/8890c22d580258851b3929ae058c196e2183c7db))
* validate step graph references and cycles ([#21](https://github.com/yildirimarda/freshen/issues/21)) ([ee61218](https://github.com/yildirimarda/freshen/commit/ee612186adf208170c939579411df17d99e8e1d8))
* wait for other package managers before updating ([#37](https://github.com/yildirimarda/freshen/issues/37)) ([b2e624a](https://github.com/yildirimarda/freshen/commit/b2e624ab3f58459409b238e38b9b47ea813d7a28))
* warn about long-running sessions an upgrade would break ([#43](https://github.com/yildirimarda/freshen/issues/43)) ([cdfb06f](https://github.com/yildirimarda/freshen/commit/cdfb06f76a484c25025de7be90b7787ccf513cfa))
* warn about processes running old toolchain versions ([#42](https://github.com/yildirimarda/freshen/issues/42)) ([fbf219a](https://github.com/yildirimarda/freshen/commit/fbf219a76f8b4d8324dc09e0fb89a09686a3cad0))
* warn about stale Node processes and in-session zellij upgrades ([#18](https://github.com/yildirimarda/freshen/issues/18)) ([1e6c8e6](https://github.com/yildirimarda/freshen/commit/1e6c8e697915b30ca68160f327dbc3149ad7b33a))
* warn only about toolchain versions mise will prune ([#54](https://github.com/yildirimarda/freshen/issues/54)) ([7b3aed3](https://github.com/yildirimarda/freshen/commit/7b3aed38b7fd885599bd9ab311a930e7d341b8c4))


### Bug Fixes

* make retry backoff interruptible and deadline-aware ([#52](https://github.com/yildirimarda/freshen/issues/52)) ([637d586](https://github.com/yildirimarda/freshen/commit/637d586279dc8543ffdf2351dd5e6d8c158bd79a))


### Miscellaneous Chores

* release 0.1.0 ([b7abcd0](https://github.com/yildirimarda/freshen/commit/b7abcd020b6688ca32e0ec8c7a9766a500862a6a))

## [Unreleased]

### Changed

- The Rust `freshen` binary is now the implementation; the zsh script has been retired.

### Removed

- The zsh implementation of the update steps. Use `freshen`; see the migration section in the README.
- The deprecated `update-all.sh` compatibility shim.
