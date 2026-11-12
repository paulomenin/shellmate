---
title: Development requirements
description: The programs a development machine needs to build shellmate, run every test, and build the documentation site.
---

Which programs you need depends on what you do. The first table is enough to build and run `cargo test`. The others add the checks CI runs.

## Build and run the tests

| Program | Version | Used for |
|---|---|---|
| Rust | 1.99.1 with `rustfmt`, `clippy` and `llvm-tools-preview` (the version the project is tested with and the dev tools need; building shellmate alone needs 1.86 or newer) | Everything. `mise install` sets it up. |
| A C compiler and linker (`cc`) | any | Linking Rust programs |
| `git` | any recent | The git context collector, the `exec` git rules, and tests |
| `bash` | 4.4 or newer | Shell glue, and the glue syntax tests |
| `fish` and `zsh` | any recent | Shell glue, the glue syntax tests, and the end-to-end suite |
| `sh`, `find`, `sed`, `tar`, `ln`, `stat` | POSIX | Test scripts and the classifier tests |

Without `git`, `fish`, `zsh` or `bash`, the tests that need them skip with a message. Set `REQUIRE_TOOLS=1` and a missing program fails the run instead, which is what CI does:

```sh
REQUIRE_TOOLS=1 cargo test --workspace --locked
```

## Run every check CI runs

| Program | Version | Used for |
|---|---|---|
| [`mise`](https://mise.jdx.dev/) | any | Installs the pinned Rust, Node and pnpm from `mise.toml` |
| `tmux` | 3.x | The end-to-end suite (`tools/e2e/run-all.sh`) |
| `python3` | 3.11 or newer | The end-to-end mock provider and `tools/bench` (standard library only) |
| `curl` | any | The end-to-end CI job |
| Node.js and pnpm | 26.13.0 and 12.17.0, pinned in `mise.toml` | The documentation site |
| [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) | 0.9.1 | `cargo cov` |
| [`cargo-mutants`](https://mutants.rs/) | 27.1.0 | `cargo mut` and `mise run mutants` |
| [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) | recent | Licenses, advisories and sources (`cargo deny check`) |


## Optional

| Program | Used for |
|---|---|
| `starship` | Runs the end-to-end suite with a prompt framework as well. Skipped when absent. |
| `ollama` | `tools/bench/evals.py --ollama`, to score models you already have. |
| `man` (man-db) | The `man` tool needs real manual pages when you try shellmate by hand. The tests use a fake `man`. |
| `gh` | Pull requests and CI runs from the terminal. |
| `stress-ng` | Putting the machine under load to reproduce a flaky test. |
| A headless Chromium | Taking a screenshot of the built site to check a diagram. |
| `musl-tools` and the musl Rust targets | Building the release binaries locally. CI installs them for the release workflow. |

## Install

With mise (Rust, Node and pnpm at the pinned versions):

```sh
mise install                    # also installs cargo-mutants, built from source
cargo install --locked cargo-llvm-cov --version 0.9.1
cargo install --locked cargo-deny
```

System packages, on Debian or Ubuntu:

```sh
sudo apt-get install -y build-essential git bash fish zsh tmux python3 curl man-db
```

On Arch Linux:

```sh
sudo pacman -S --needed base-devel git bash fish zsh tmux python curl man-db
```

macOS and musl builds have not been tested. Other platforms are not covered here.

## Check what you have

```sh
for p in git bash fish zsh tmux python3 curl node pnpm cargo-llvm-cov cargo-mutants cargo-deny; do
  command -v "$p" >/dev/null && echo "ok       $p" || echo "MISSING  $p"
done
```

Then follow [Contributing](../contributing/) for the checks to run before a pull request.
