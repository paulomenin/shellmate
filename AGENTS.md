# AGENTS.md

Guidance for AI agents (and humans) working on **shellmate**, an open-source (MIT) Rust CLI companion for the shell. You type a question at your prompt, press a key, and the answer streams below it. A suggested command is placed in your input line and is never run for you.

Read this file first. Task-specific playbooks are in `.agents/skills/`.

## Direction (decisions already made, do not reopen without being asked)

- **Thin glue, logic in Rust.** fish, zsh and bash get a small glue template (`crates/shells/`). Anything testable belongs in Rust.
- **Buffer-first.** Suggestions go into the input line. Nothing the model produces runs without a person (INV-1).
- **Oneshot runtime.** Every request is a new short-lived process (`shellmate invoke`). A daemon is planned, not built.
- **Everything that can be a plugin is one**, behind `shellmate-api`: providers, deciders, middleware, stores and tools. The core keeps the decisions that protect the user (approval, pre-approval, taint, escaping), so a tool can only ask for stricter handling.
- **Redaction is middleware** and fails closed. A remote provider is refused without an active redactor (INV-2).
- **Config is TOML.** Secrets are only `env:NAME`, `file:PATH` or `cmd:PROGRAM ARGS` references, never literal values. `cmd:` is a core scheme, fetched on first use (`secret_cmd.rs`, `engine/lazy.rs`).
- **Names:** binary `shellmate`, optional alias `sm`, default shortcut `alt+m`. `generate` is command-only (`alt+i`), `explain` is `alt+j`, `new-conversation` is `alt+k`, `capture` is `alt+r` (`alt+e` in bash). The built-in default is only `alt+m`; the config written by `setup` binds all of them.
- **Website:** Astro Starlight, pnpm, fully static. Public docs live in `website/src/content/docs/docs/`.
- **Non-goals:** a GUI, a full-screen chat client, telemetry of any kind, hard dependencies on one terminal emulator.

## Security invariants

These hold regardless of configuration. A change must not weaken them, and any change near them needs a test that shows the dangerous input is refused. See `.agents/skills/security-sensitive-change/SKILL.md`.

- **INV-1** Executing a command for the model needs human approval at a prompt only a person can answer (no terminal means deny). The classifier keeps its veto over `preapproved` rules. Plugins can only make approval stricter.
- **INV-2** A request reaches a network provider only after the full outbound chain. Remote providers need redaction.
- **INV-3** Plugins get only the permissions they declare and the user grants (matters once external plugins exist).
- **INV-4** Untrusted content (captured output, file text, command output, commit subjects) taints the request and suspends pre-approvals for it.
- Commands run directly, never through a shell, with a sanitized environment, a timeout and capped output.
- The audit log records what shellmate did, never prompts, answers or tool results.

## Repository map

| Path | What |
|---|---|
| `crates/shellmate-api` | The plugin API: traits, types, `Registry`. Plugins depend only on this. |
| `crates/shellmate-core` | Engine (`engine/`, one file per concern; the host that runs a tool and enforces approval, taint and escaping is `engine/tool_calls/`), config, prompts, conversations, tools, classifier (`command/`), validator, audit, rendering, and the ports for env, clock and files (`ports.rs`), and the presenter that turns answers, notices, notes and errors into terminal bytes (`present/`) |
| `crates/shellmate-builtin` | The first-party plugins, one module each behind a cargo feature: `provider_openai_compatible`, `decider_systemone`, `middleware_redact`, `store_tmpfs`, and the tools `tool_man`, `tool_exec`, `tool_read`. Registers them. |
| `crates/shellmate` | The binary: `cli.rs`, one module per command in `commands/`, setup, doctor and other admin commands in `admin/`, init, approval prompt, tty |
| `crates/shells/` | Glue templates for fish, zsh, bash |
| `tools/shellmate-testing` | Mock HTTP/SSE servers, a `FakeProvider` and an in-memory conversation store, `MemoryStore` (dev-dependency) |
| `tools/e2e` | Real fish, zsh, bash in tmux against a mock provider |
| `tools/bench` | `perf.py` (process performance) and `report.py`; `evals/` is the model evaluator |
| `tools/mutants.sh` | Mutation testing |
| `website/` | The documentation site |
| `docs/` | **Git-ignored** internal design notes. They may be missing in a fresh clone. Code comments that cite `docs/...` refer to them. The public docs are the source of truth. |

## Commands

```sh
mise install                                   # pinned Rust 1.99.1, Node, pnpm
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
REQUIRE_TOOLS=1 cargo test --workspace --locked   # missing git/fish/zsh/bash fails instead of skipping (CI does this)
tools/e2e/run-all.sh                           # when you touch glue, keys or terminal handling
cargo cov                                      # coverage, HTML in target/llvm-cov/html
cargo mut -p shellmate-core -f 'crates/shellmate-core/src/command/*.rs'   # mutation testing, one file
mise run mutants                               # the security modules, about 30 minutes
python3 -m unittest discover -s tools/bench          # performance tooling tests
python3 -I -m unittest discover -s tools/bench/evals  # model evaluator tests (needs the image: see its README)
(cd website && pnpm install && pnpm build)     # docs site
```

Run fmt, clippy and tests before every commit. Both must be clean.

## Conventions

- **Toolchain.** Built and tested with Rust 1.99.1 (`mise.toml`). The minimum Rust to build shellmate is 1.86 (`rust-version`, set by the locked dependencies; the `msrv` CI job builds with it), so do not use newer language or library features than that in shipped code. Dev tools are at `cargo-llvm-cov 0.9.1`, `cargo-mutants 27.1.0`, `proptest 1.12`, Node 26.13.0, pnpm 12.17.0. Update them deliberately, one at a time (see `.agents/skills/dependency-update/SKILL.md`).
- **Errors.** Use `shellmate_api::Error::new("area.code", "message").with_hint("what to do")`. Messages say what happened and what the user can do.
- **Dependencies.** Prefer none. A new one must pass `deny.toml` (licenses, advisories, sources) and should not raise the MSRV. Plugin crates depend on `shellmate-api`, never on `shellmate-core`.
- **Latency.** A run takes about 8 ms before network traffic, with a 30 ms budget enforced in CI. Do not add blocking work to the start-up path.
- **Look at the machine through `Host`.** Core code reads environment variables, the clock and files through `Host` (`ports.rs`), not through `std::env`, `SystemTime` or `std::fs`. Starting programs, the `read` tool's path checks and writes (audit, capture, stores) stay on the real system on purpose. See the module comment.
- **Show text through a `Presenter`.** The engine says what happened as values with a tone (`Notice` while a request runs, `Remark` after the answer), never as strings with symbols in them. `present/` turns them into bytes; `PlainPresenter` is the text-only fallback. Escaping of text that is not ours stays in the presenter, and only the theme writes escape sequences (a property test checks it). Colour and markdown follow `[ui]` and `NO_COLOR`/`CLICOLOR`/`CLICOLOR_FORCE`/`TERM` (`present/level.rs`). Today this covers what `invoke` prints (answer, notices, notes, errors, spinner), `doctor`, the approval prompt, command error lines and the report commands (`check`, `config check|show`, `context`, `audit`, `conversation`, `setup`), which take a `Presenter` from `commands::ui::stdout()` and use `status` and `label`. `capture` and `shell-init` print raw text and stay unstyled. `shellmate render FILE` previews a look.
- **Short internal limits** (git, the syntax check, `man`, the grace period after a `cmd:` secret program exits) must go through `limits::scaled`, so `SHELLMATE_TIME_SCALE` can stretch them.
- **User-visible change?** Add a line under "Unreleased" in `CHANGELOG.md` and update the page in `website/` that describes it.
- **Docs voice.** Plain, direct sentences, no marketing, no em dashes. Match the neighbouring pages.
- **Commits.** Small and logical, with a message that says why. Conventional prefixes are used (`test:`, `docs:`, `build:`, `fix:`, `feat:`). Do not push, tag or publish unless asked. `mise.lock` may show local changes; leave it out of your commit unless you changed the tools.
- **Never** pull large models, make network calls that cost money, or write secrets into files, logs or tests unless the user asks.

## Tests

The suite needs no model, no network and no special terminal. A mock provider stands in for the model, so tests are deterministic.

- Unit tests sit next to the code. Property tests are in `tests/properties.rs` of `shellmate-core` and `crates/shellmate-builtin/tests/` for the redactor (proptest, `failure_persistence: None`). Binary tests are in `crates/shellmate/tests/` and drive the real binary against mock servers. `approval_pty.rs` runs the approval prompt in a pseudo-terminal.
- No wall-clock assertions. Use `SHELLMATE_TIME_SCALE` (`.cargo/config.toml` sets 5). Do not call `std::env::set_var` in a test: pass a `MapEnv`, `ManualClock` or `MemFs` through `Host`.
- Tests that run the binary pin the colour variables (`NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE`, and `TERM` for the pty tests), so a developer's own settings cannot change a result.
- A unit-test block that outgrows its file goes in a sibling `tests.rs` (`#[cfg(test)] mod tests;`), where it can still see private items.
- Never skip silently. Use `have(...)` (`needs.rs`) or `have_tool(...)` (`tests/common`) so `REQUIRE_TOOLS=1` turns a missing tool into a failure.
- For security-relevant code, test the refusal, not only the success.
- Test through public behaviour. Do not pin incidental wording.
- Coverage says a line ran. Mutation testing says a test would notice if it were wrong. After touching `command/`, `tools/`, `validate/`, `engine/tool_calls/`, `present/`, `render.rs`, `sanitize.rs`, `secrets.rs`, `secret_cmd.rs` or the redactor, run it and kill the survivors, or exclude one in `.cargo/mutants.toml` with a written reason.

More in `.agents/skills/test-and-measure/SKILL.md`.

## Playbooks

| Task | Read |
|---|---|
| Add a provider, decider, middleware or store | `.agents/skills/add-plugin/SKILL.md` |
| Add a tool the model can call (web search, MCP, ...) | `.agents/skills/add-tool/SKILL.md` |
| Change the shell glue, a key binding, or support a shell | `.agents/skills/add-shell-or-key-binding/SKILL.md` |
| Add or change a config key | `.agents/skills/add-config-setting/SKILL.md` |
| Change or add documentation, the sidebar, a diagram | `.agents/skills/update-docs/SKILL.md` |
| Write tests, check coverage, run mutation tests, run benchmarks | `.agents/skills/test-and-measure/SKILL.md` |
| Fix a flaky or load-sensitive test | `.agents/skills/investigate-flaky-test/SKILL.md` |
| Add an eval case, evaluate a model, publish the models report | `.agents/skills/add-eval-case-and-model-report/SKILL.md` |
| Change a prompt | `.agents/skills/change-prompt/SKILL.md` |
| Touch the classifier, approval, redaction, tools, taint or config secrets | `.agents/skills/security-sensitive-change/SKILL.md` |
| Triage or reproduce a bug report | `.agents/skills/triage-bug-report/SKILL.md` |
| Review a change (or your own, before finishing) | `.agents/skills/review-pr/SKILL.md` |
| Update dependencies, the toolchain or pinned dev tools | `.agents/skills/dependency-update/SKILL.md` |
| Package or release (nothing is published yet; a full release playbook is postponed) | `.agents/skills/packaging/SKILL.md` |

The skills are plain Markdown with front matter, so any agent can read them. `.claude/` is git-ignored here, so they are not stored there. To use them as Claude Code skills, symlink: `mkdir -p .claude/skills && ln -s "$PWD"/.agents/skills/* .claude/skills/`.

## Known gaps (do not claim otherwise)

No release has been published and the CI jobs have not run on GitHub yet. macOS, musl, powerlevel10k and older shells are untested. Mutation testing covers `command/`, `tools/`, `validate/`, the redactor, the three tool plugins, `engine/tool_calls/`, `present/`, `render.rs`, `sanitize.rs`, `secrets.rs` and `secret_cmd.rs` only. The parsers are not fuzzed. The e2e suite is not in the coverage numbers. See the roadmap page for the full list.
