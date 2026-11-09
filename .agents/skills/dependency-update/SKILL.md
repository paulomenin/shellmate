---
name: dependency-update
description: Update Rust or JavaScript dependencies, the pinned toolchain, or the pinned dev tools (proptest, cargo-llvm-cov, cargo-mutants, Node, pnpm) in shellmate without breaking the MSRV, the license policy or CI.
---

# Update dependencies and tools

Less is better: every dependency is attack surface and compile time. Prefer removing one to updating it. Do not update just because something is newer.

## What is pinned, and why

| Pin | Where | Why |
|---|---|---|
| Rust 1.99.1 (rustfmt, clippy, llvm-tools-preview) | `mise.toml` | The toolchain the workspace is tested with and that the dev tools need. The minimum to **build shellmate** is **1.86** (`rust-version` in `Cargo.toml`, set by the locked dependencies) and must not rise silently |
| Node 26.13.0, pnpm 12.17.0 | `mise.toml`, `website/.nvmrc`, `website/package.json` (`packageManager`) | Keep the three in step. Astro 7 needs Node 22.23.4 or newer |
| `proptest 1.12` | workspace `Cargo.toml` | Dev only. It needs a newer Rust than the 1.86 minimum, so running the tests needs the mise toolchain |
| `cargo-llvm-cov 0.9.1` | `mise.toml` comment, `tools/README.md`, docs | Installed with `cargo install --locked`. CI installs the latest through `taiki-e/install-action` |
| `cargo-mutants 27.1.0` | `tools/README.md`, `tools/mutants.sh` message, docs | Installed with `cargo install --locked` |
| `Cargo.lock` | committed | Everything builds with `--locked` |
| `mise.lock` | committed | Exact tool versions and checksums |
| `website/pnpm-lock.yaml` | committed | |

CI installs Rust with `dtolnay/rust-toolchain@stable` for most jobs, and the `msrv` job builds the workspace with Rust 1.86.0 (`cargo build --workspace --locked`). That job also fails if its toolchain differs from `rust-version` in `Cargo.toml`, so when you raise the minimum, change both.

## Rust crates

```sh
cargo update -p <crate> --precise <version>      # one crate at a time
cargo tree -i <crate>                            # who needs it
cargo deny check                                 # licenses, advisories, bans, sources (also runs in CI)
cargo +1.86.0 build --workspace --locked         # the minimum (the msrv CI job); build only, tests need the mise toolchain
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

- A crate whose new version raises the minimum Rust above 1.86 (for code that ships in the binary) is not acceptable without the user's decision. Pin the older one and record why in a comment next to the dependency. Dev-only dependencies (`proptest`) may need the newer mise toolchain.
- `deny.toml` allows a fixed license list and denies wildcard versions and unknown registries or git sources. Do not add a license to the allow list to make a build pass without asking.
- A security advisory: update the crate, or, if no fixed version exists, say so and do not silence the advisory without a written reason and expiry.
- Dev-only dependencies (proptest, test helpers) should stay out of the released binary. Check with `cargo tree -e normal -p shellmate`.
- Watch binary size and the 30 ms start-up budget when a dependency changes. Run `cargo build --release -p shellmate` and `tools/bench/perf.py --quick`.
- Feature flags: keep `default-features = false` where the workspace already does, and add only what is needed (see `clap`, `proptest`).

## The toolchain

Raising Rust is a decision, not a chore. If asked: change `mise.toml`, run `mise install`, and then update the dev tools (`proptest`, `cargo-llvm-cov`, `cargo-mutants`) one at a time, each in its own commit with the test suite and a mutation run. Update the version numbers in `tools/README.md`, `AGENTS.md`, `website/src/content/docs/docs/contributing.md` and `development-requirements.md`. Raising the **minimum Rust** (`rust-version`) needs a `CHANGELOG.md` line.

## Website (Node)

```sh
cd website
pnpm outdated
pnpm update <package>            # then review the lockfile diff
pnpm build                       # must pass; also look at the pages in a browser, Starlight and mermaid changes show up visually
```

pnpm 12 rejects packages published within its `minimumReleaseAge` window (a supply-chain protection). If `pnpm install` fails with `ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION`, pin the dependency to an older release (`mermaid` is pinned to `12.2.0` for this reason) or wait. Do **not** add `minimumReleaseAgeExclude` to get past it. A burst of fresh releases of old, stable packages is worth a look before trusting it.

Starlight, Astro and `astro-mermaid` release often; update them together and read their changelogs. Do not add client-side JavaScript to the landing page for convenience: the site is static and small.

## After any update

All the checks above plus `tools/e2e/run-all.sh` if a dependency touches the terminal, tokio, TLS or HTTP. Commit lockfile changes with the manifest change that caused them, one logical update per commit, with the reason (a fix, an advisory, a needed feature), not just "bump".
