# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- Managed by release-please from Conventional Commits. Do not edit by hand. -->

## [0.2.0](https://github.com/yildirimarda/freshen/compare/v0.1.0...v0.2.0) (2026-10-24)


### Features

* add native Docker Engine runtime on Linux ([#104](https://github.com/yildirimarda/freshen/issues/104)) ([0e8d067](https://github.com/yildirimarda/freshen/commit/0e8d067cb510b2e062740c4a2a5802a0ccfd79c5))
* decide per cask whether the upgrade needs --greedy ([#84](https://github.com/yildirimarda/freshen/issues/84)) ([74f199e](https://github.com/yildirimarda/freshen/commit/74f199e9b6a361fc597c9918266fedcdb3a50f3c))
* detect the active container runtime ([#97](https://github.com/yildirimarda/freshen/issues/97)) ([7d6bf8a](https://github.com/yildirimarda/freshen/commit/7d6bf8a29e5f3c60674b2d670d636e0a409a657c))
* explain a denied Automation request when quitting apps ([#82](https://github.com/yildirimarda/freshen/issues/82)) ([b06e5cb](https://github.com/yildirimarda/freshen/commit/b06e5cb26df5581c97a80d5755f882f935af0af3))
* extend real-tools CI proposal to apt and Windows ([#75](https://github.com/yildirimarda/freshen/issues/75)) ([5013d63](https://github.com/yildirimarda/freshen/commit/5013d63d00f5da4c28565ff454708bce08371447))
* guard screen, byobu, dtach, abduco and mosh sessions ([#63](https://github.com/yildirimarda/freshen/issues/63)) ([f6ca4ed](https://github.com/yildirimarda/freshen/commit/f6ca4eda6eca679e942f3ded22ba6a8d53f9810d))
* handle Docker Desktop cask upgrades on macOS ([#98](https://github.com/yildirimarda/freshen/issues/98)) ([9a695af](https://github.com/yildirimarda/freshen/commit/9a695afb65e284b502b0cbd9cbb260441bf30377))
* handle dual-architecture Homebrew installs ([#95](https://github.com/yildirimarda/freshen/issues/95)) ([33d3ed3](https://github.com/yildirimarda/freshen/commit/33d3ed387c925c736037cf6e3fc1cec45843eb64))
* quit cask apps named only in the uninstall stanza ([#87](https://github.com/yildirimarda/freshen/issues/87)) ([7061f6d](https://github.com/yildirimarda/freshen/commit/7061f6dbd56806d520705baaef9456973e59e73b))
* reopen the apps the way they were before the upgrade ([#89](https://github.com/yildirimarda/freshen/issues/89)) ([d1d8b78](https://github.com/yildirimarda/freshen/commit/d1d8b788e91f9d08a4f49c56987611c7c44385da))
* report outdated uv and mise tools in check mode ([#67](https://github.com/yildirimarda/freshen/issues/67)) ([6e849f2](https://github.com/yildirimarda/freshen/commit/6e849f21dbd13296adf94de908e92b41f441d174))
* respect pinned Homebrew formulae ([#85](https://github.com/yildirimarda/freshen/issues/85)) ([9df5981](https://github.com/yildirimarda/freshen/commit/9df59812d617b6886d3ec0a42b786aaf9f34f12c))
* restart Homebrew services for upgraded formulae ([#86](https://github.com/yildirimarda/freshen/issues/86)) ([b28aa95](https://github.com/yildirimarda/freshen/commit/b28aa956544eee9bbffa675e2784416a9c26784a))
* run check queries concurrently with bounded jobs ([#68](https://github.com/yildirimarda/freshen/issues/68)) ([35573f1](https://github.com/yildirimarda/freshen/commit/35573f1cce783c2fa92201e68e4d9fd3db1d4f48))
* run Rust CI on macOS, Ubuntu and Windows ([#73](https://github.com/yildirimarda/freshen/issues/73)) ([a536246](https://github.com/yildirimarda/freshen/commit/a5362464f1d51c8037973b73fc42aba94969f14e))
* run the commands freshen parses in a neutral locale ([#81](https://github.com/yildirimarda/freshen/issues/81)) ([8021b21](https://github.com/yildirimarda/freshen/commit/8021b215eae7a62c4a60cef618bcf5eb80f55c5d))
* stop and start standalone Lima instances around a lima upgrade ([#102](https://github.com/yildirimarda/freshen/issues/102)) ([a73b0c8](https://github.com/yildirimarda/freshen/commit/a73b0c8dbbd7dbb335ce30cdf327bf7ee7d468f6))
* support per-step flags, include/exclude lists and app lists ([#80](https://github.com/yildirimarda/freshen/issues/80)) ([e099f15](https://github.com/yildirimarda/freshen/commit/e099f15d62f49be1d257e7e9ca4ba6f8b9b1845a))
* wait for apps that update themselves instead of quitting them ([#88](https://github.com/yildirimarda/freshen/issues/88)) ([ce8b9eb](https://github.com/yildirimarda/freshen/commit/ce8b9eb8c8a038c13c5309298877d91c8ac36cb9))


### Bug Fixes

* make cask app roots overridable so macOS tests ignore the real /Applications; windows clippy ([1030c44](https://github.com/yildirimarda/freshen/commit/1030c44cfe698a404f52c68c4aeec32d6070dad7))
* protect detected terminal when quitting casks ([#69](https://github.com/yildirimarda/freshen/issues/69)) ([57c6a48](https://github.com/yildirimarda/freshen/commit/57c6a4819ea469a03c4f8cc3b1bb0870b7c4ecfb))

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
