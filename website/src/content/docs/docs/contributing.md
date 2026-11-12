---
title: Contributing
description: Set up a development environment, run the checks, and know what a change has to keep true.
---

Thank you for helping. This page covers how to build and test shellmate, what the test suite expects of a change, and the rules the code has to keep. For how the pieces fit together, read [Architecture](../concepts/architecture/) first.

## Set up

The tool versions are pinned in `mise.toml` (Rust 1.99.1, Node, pnpm). With [mise](https://mise.jdx.dev/):

```sh
mise install
cargo build
```

Without mise, use a Rust toolchain with `rustfmt` and `clippy`. Building shellmate needs Rust 1.86 or newer. The tests and the dev tools need a newer one, so use the version in `mise.toml`. CI tests with the current stable and also builds with 1.86 (the `msrv` job).

The shell tests use real programs. Install `git`, `fish`, `zsh` and `bash`. The end-to-end suite also needs `tmux`, `python3` and (optionally) `starship`. The full list, with versions and install commands, is in [Development requirements](../development-requirements/).

## Before you open a pull request

These are the checks CI runs. Run them locally first:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
tools/e2e/run-all.sh          # real fish, zsh and bash in tmux; needed when you touch shell glue or key handling
```

Also:

- Add a line to `CHANGELOG.md` under "Unreleased" for anything a user would notice.
- Update the documentation page that describes the behavior you changed. Run `pnpm build` in `website/` if you touched the site.
- Keep commits focused, with a message that says why, not only what.

## Tests

The suite runs with no model, no network and no real terminal state. A mock provider stands in for the language model, so every test is deterministic.

| Layer | Where | What it covers |
|---|---|---|
| Unit | `#[cfg(test)]` in each crate; a long test block goes in a sibling `tests.rs` | Parsers, the classifier, rules, the redactor, the validator, the tool decisions |
| Property | `tests/properties.rs` in `shellmate-core` and the redactor's tests in `shellmate-builtin` | Invariants over generated input: nothing leaks, accepted commands contain no shell syntax, streaming does not depend on how the text is split |
| Integration | `crates/shellmate/tests/` | The real binary against mock servers, including the approval prompt in a pseudo-terminal (`approval_pty.rs`) |
| End to end | `tools/e2e/` | Real fish, zsh and bash in tmux, against a mock provider |

Rules for writing tests:

- **Test behavior, not wording.** Assert on what the user or the next step sees. Do not pin incidental text.
- **Do not touch the machine from a unit test.** The core reads the environment, the clock and files through `Host` (`ports.rs`): pass a `MapEnv`, a `ManualClock` or a `MemFs` instead of calling `std::env::set_var` or writing temp files. A fake provider, mock servers and an in-memory conversation store are in `tools/shellmate-testing`.
- **No clocks.** Do not rely on wall-clock timing. The short internal limits (git, the syntax check, `man`) stretch with `SHELLMATE_TIME_SCALE`, which `.cargo/config.toml` sets to 5 for you.
- **Never skip silently.** A test that needs `git`, `fish`, `zsh` or `bash` should use the `have` helper (in `needs.rs` for unit tests, `have_tool` in `tests/common`). Set `REQUIRE_TOOLS=1` and a missing tool becomes a failure; CI does this.
- **Test the refusal.** For anything security-relevant, write the test that shows the dangerous input is rejected, not only that the good input works.
- Property tests use `proptest`.

### Coverage and mutation testing

Line coverage says code ran. Mutation testing says a test would notice if the code were wrong.

```sh
cargo install --locked cargo-llvm-cov --version 0.9.1
cargo cov                       # HTML report in target/llvm-cov/html (or: mise run coverage, which also prints a summary)

mise install                    # installs cargo-mutants at the pinned version
mise run mutants                # the security modules, about 30 minutes
cargo mut -p shellmate-core -f 'crates/shellmate-core/src/command/*.rs'   # one file
```

Run mutation testing after changing `command/`, `tools/`, `validate/`, `engine/tool_calls/`, the tool plugins, `present/`, `render.rs`, `sanitize.rs` or the redactor. Every mutant that survives is either a missing test or a change that cannot be observed. Add the test, or add the mutant to `exclude_re` in `.cargo/mutants.toml` with the reason. See `tools/README.md` for how to read the results.

## What a change must keep true

The [security model](../concepts/security-model/) is enforced in code and covered by tests. A change must not weaken it. In practice:

- **Nothing the model produces runs without a human (INV-1).** Suggested commands go to the input line and are never executed. `exec` and `read` ask first, at a prompt that only a person at the terminal can answer. No terminal means deny.
- **The classifier keeps its veto.** A pre-approval rule never overrides a command the classifier flags. Plugins can make approval stricter, never looser.
- **Remote providers need redaction (INV-2).** Outbound text goes through the redaction chain, which fails closed.
- **Untrusted content is data (INV-4).** Command output, file text and captured output are labeled as data, and once they enter a request, pre-approvals are suspended for it.
- **Secrets stay out of the config.** Credentials are `env:` or `file:` references, never literal values, and nothing in the audit log records prompts or answers.
- **Commands never run through a shell.** `exec` runs a single program with a sanitized environment, a timeout and capped output.

If a change touches one of these, say so in the pull request and show the test that proves it still holds.

## Project layout

```text
crates/shellmate-api            the plugin API
crates/shellmate-core           engine (engine/), config, tools, classifier, validator, ports
crates/shellmate-builtin        the first-party plugins (one module each) and their registry
crates/shellmate                the binary (commands/, admin/, approval prompt)
crates/shells/                  glue templates for fish, zsh and bash
tools/                          e2e suite, benchmarks, mutation testing, test helpers
website/                        this site (Astro Starlight, pnpm)
```

The shell glue stays thin. Behavior belongs in Rust, where it can be tested without a terminal.

## The documentation site

The site lives in `website/` and is built with Astro Starlight.

```sh
cd website
pnpm install
pnpm dev        # http://localhost:4321/docs/
pnpm build
```

Pages are Markdown in `website/src/content/docs/docs/`. Add a new page to the sidebar in `website/astro.config.mjs`. Links between pages are relative.

## Working with a coding agent

The repository has an `AGENTS.md` at its root and playbooks in `.agents/skills/` (adding a plugin, a tool, a config key or a key binding; updating the docs; testing, measuring and evals; changing prompts or security-sensitive code; triage, review and dependency updates). They are plain Markdown, so they are worth reading as a human too. An agent's change goes through the same checks and review as yours, and the rules in "What a change must keep true" apply to it without exception.

## License

shellmate is MIT licensed. By contributing you agree that your work is released under the same license.
