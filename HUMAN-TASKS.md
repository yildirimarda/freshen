# Human tasks

Things the autonomous loop cannot or must not do. The loop only reads `PLAN.md`; this file is for you.

## Apply workflow proposals

The agent cannot edit `.github/workflows/`. For CI items it writes the workflow under `ci-proposals/` and ticks the item. To activate a proposal:

```bash
git mv ci-proposals/<name>.yml .github/workflows/<name>.yml
git commit -m "ci: apply <name> proposal" && git push
```

Pending proposals:

- [ ] `ci-proposals/rust.yml` (separate Rust job, MSRV check, cargo-deny). Apply after the plan item "Make `tests/run-tests.sh` accept each proposed workflow..." has merged.
- [ ] `ci-proposals/real-tools.yml` (read-only checks against real Homebrew, Chocolatey, winget and apt), once the agent has proposed it.

## Accounts and credentials for Milestone 11 (distribution)

The agent can prepare workflows and manifests, but only you can create these:

- [ ] crates.io API token, stored as the repository secret `CARGO_REGISTRY_TOKEN`
- [ ] Apple Developer ID certificate and notarization credentials (repository secrets) for signing the macOS binary
- [ ] Windows code-signing certificate (repository secret)
- [ ] Repository `yildirimarda/homebrew-tap` for the Homebrew formula, and a Scoop bucket repository
- [ ] A pull request to `microsoft/winget-pkgs` for the winget manifest
- [ ] An AUR account and SSH key for the PKGBUILD
- [ ] `AUTOMERGE_TOKEN` repository secret (done) and its renewal date

## Reviews

- [ ] Merge `plan`-labelled pull requests yourself (`gh pr list --label plan`)
- [ ] Review the release pull request before merging it (`gh pr list --search release`)
- [ ] Run the checklist in `docs/TESTING.md` before every minor release (once it exists)
