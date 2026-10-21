# Human tasks

Things the autonomous loop cannot or must not do. The loop only reads `PLAN.md`; this file is for you.

## Apply workflow proposals

The agent cannot edit `.github/workflows/`. For CI items it writes the workflow under `ci-proposals/` and ticks the item. To activate a proposal:

```bash
git mv ci-proposals/<name>.yml .github/workflows/<name>.yml
git commit -m "ci: apply <name> proposal" && git push
```

Pending proposals:

- [ ] `ci-proposals/rust.yml` — the `rust` workflow with the Windows leg blocking again. The `continue-on-error` workaround for the Windows unit tests is gone now that they pass, so the proposal drops the line. The workflow is already applied as `.github/workflows/rust.yml`, so replace it instead of moving it:

  ```bash
  git mv -f ci-proposals/rust.yml .github/workflows/rust.yml
  git commit -m "ci: make the windows leg of the rust workflow block again" && git push
  ```

- [ ] `ci-proposals/real-tools.yml` (read-only checks against real Homebrew, Chocolatey, winget and apt), once the agent has proposed it and the apt (Milestone 5) and winget/Chocolatey (Milestone 6) steps exist. The new jobs fail closed on a missing or skipped step, so applying the proposal before those steps land turns the workflow red by design.

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
