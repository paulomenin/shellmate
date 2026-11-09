---
name: packaging
description: Package shellmate for distribution (static release binaries, AUR, Homebrew, Nix). Only useful once a release exists. Postponed work: no release or package has been published yet.
---

# Packaging

**Status: nothing is published yet.** There is no release tag, the release workflow has not run on a real tag, and the AUR, Homebrew and Nix recipes are planned only (see the roadmap). Do not claim any package exists. Do not create tags, releases or packages unless the user asks. This file records what is known so the work can start quickly, and should be revised after the first release. A fuller `release` playbook is postponed.

## What exists

- `.github/workflows/release.yml` runs on tags `v*`. It builds `shellmate` with `--release --locked` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, packs `shellmate-<tag>-<target>.tar.gz` (binary, `LICENSE`, `README.md`) with a `.sha256`, and attaches them to a GitHub release.
- The workspace is MIT licensed. `cargo-deny` enforces dependency licenses in CI.
- Install docs: `website/src/content/docs/docs/start/install.mdx`.

## Constraints for any package

- The binary is `shellmate`. An optional `sm` alias is the user's choice; a package must not install a command named `sm` unless it checks that nothing else owns that name.
- It must build with `--locked` on Rust 1.86 or newer. No network at build time other than fetching the locked crates.
- Shell integration is installed by the user (`shellmate setup`, `shellmate install-shell`). A package must not edit the user's rc files. It may ship the glue output location or completions only if they are plain files, not hooks.
- No telemetry and no network calls at install time.
- Static musl builds are untested on real machines; macOS builds are untested. Do not claim support.

## Before the first release (checklist to turn into a skill later)

1. All CI jobs have run green on GitHub at least once (rust, shells, perf, coverage, deny, website).
2. A `perf.py` baseline for the version and the models page are refreshed (`.agents/skills/test-and-measure/SKILL.md`, `.agents/skills/add-eval-case-and-model-report/SKILL.md`).
3. `CHANGELOG.md` "Unreleased" becomes the version section. The roadmap's "Known gaps" are honest.
4. The version in `Cargo.toml` is bumped (`[workspace.package] version`), `Cargo.lock` updated.
5. The release is made by the user. Verify the tarball: checksum, `shellmate --version`, `shellmate doctor` in a clean container, a real fish, zsh and bash session.

## Recipes (when asked)

Write each as a separate file outside this repository's crates (for example a `packaging/` directory, or the distribution's own repository), generated from the release tarball URL and its checksum:

- **AUR:** a `PKGBUILD` for the source (`cargo build --release --locked`) and, optionally, a `-bin` package from the tarball. `check()` runs `cargo test --workspace --locked`; tests that need fish, zsh and bash should be skipped or the shells added to `checkdepends`.
- **Homebrew:** a formula building from source with `cargo install` or using the tarball; `test do` runs `shellmate --version`. Do not add macOS claims until it has been tried on macOS.
- **Nix:** a derivation with `buildRustPackage` and the lockfile hash.

Test every recipe in a clean build environment, and report what you ran and what you could not.
