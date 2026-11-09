---
title: Status and roadmap
description: What works today, what is untested, and what is planned.
---

shellmate is in **early development**. There is no release yet.

## Works today

- `ask`, `generate` and `explain` in fish, zsh and bash, streamed as they are generated, with cancel by `Ctrl-C`.
- OpenAI-compatible providers (Ollama, LM Studio, OpenRouter, OpenAI) with fallback chains.
- Context levels, reversible redaction that fails closed, `--dry-run`.
- Follow-up conversations per shell session on tmpfs, with the audience rule for remote providers.
- The read-only `man` tool behind a classifier and policy.
- The `exec` tool: the model can ask to run a simple command, you approve each one at a prompt, a rule-based classifier forces extra care for risky ones, and your own `preapproved` rules skip the prompt for safe lookups.
- The `read` tool (one text file from the working directory, asked first, credentials never), plus project-type and git context collectors.
- Checks on every suggested command: the line parses in your shell, programs exist, risk notes. Optionally a decision model (any `/v1/systemone` endpoint) or the LLM judges whether the command matches your request.
- `shellmate run -- <command>` and the `capture` shortcut: wrap a command, and your next question can use its output (private, bounded, redacted, and it taints the request).
- `setup`, `doctor`, `install-shell`, `config show/init/check`, `audit`, `--explain`, `--timings`, `check`, `context`.
- Benchmark tooling: process performance with budgets and baseline comparison, and model evaluations scored by fixed rules, with record and replay and ready-to-publish reports (see [Benchmarks](../../benchmarks/)).
- A test suite that runs with no model: unit, property and integration tests (including the approval prompt in a pseudo-terminal), and a real fish, zsh and bash suite. Coverage (`cargo cov`) and mutation testing (`mise run mutants`) are set up. See [Contributing](../../contributing/).
- CI workflows and a release workflow for static Linux binaries (written, not yet exercised on a published tag).

## Known gaps

- No published releases and no packages (AUR, Homebrew and Nix recipes are planned).
- Untested: powerlevel10k, older shells, macOS and musl builds.
- The interactive `setup` prompts and the remote-provider consent screen have only been exercised through `--yes`. Setup does not test the connection or list models yet.
- The styled Markdown renderer is not built; answers are rendered as plain wrapped text.
- Mutation testing covers the classifier, the tools, the validator, the redactor, the engine's tool decisions and the presenter; the rest of the core has not been mutation tested. The parsers are property tested but not fuzzed, and the shell suite is not part of the coverage numbers.
- The coverage and mutation CI setup has not yet run on GitHub.
- The audit log is part of the core and does not yet record every event planned (for example config loading and redaction counts).

## Planned

| Area | Idea |
|---|---|
| Command execution | Pipelines in `exec`, a fish-aware grammar, and `deny_patterns`. |
| Quality | Execution-based eval cases (run the command in a sandbox), a larger case set, and prompt-change gating on the eval diff. Fuzzing the parsers, mutation testing of the remaining modules, and a scheduled mutation run in CI. |
| Config | Per-directory profiles, for example a stricter provider chain under a client's directory. |
| Secrets | A keyring plugin kind (Vault, KWallet over D-Bus, TPM) for when external plugins exist; `keyring:` today only runs `secret-tool` or `security`. A `cmd:` secret of a decision model or a plugin is fetched at start-up, not on first use. |
| Runtime | An optional background daemon. |
| Providers | More native providers, and a built-in local model via candle. |
| Tools | A web search tool and an MCP client, both behind the same approval model. |
| Conversations | Summaries for long histories, interrupted turns, a file-backed store. |
| Platforms | macOS, then more. |

## Non-goals

A GUI, a full-screen chat client, telemetry of any kind, and hard dependencies on a particular terminal emulator.

## Follow along

The code, issues and discussions live at <https://github.com/paulomenin/shellmate>. shellmate is MIT licensed.
