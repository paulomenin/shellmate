---
name: review-pr
description: Review a change to shellmate (pull request, branch or diff) against this repository's invariants, test rules, docs and dependency policy. Use when asked to review, or before declaring your own change done.
---

# Review a change

Read the diff, then go through these in order. Report findings with file and line, most severe first, and say what you checked and did not check. Do not invent problems to look thorough. If the change is fine, say so.

For a general correctness pass, the `code-review` skill applies as well. This list is what is specific to shellmate.

## 1. Safety (block the change if any of these fails)

- Could the model, or text the model read, now cause something to run, be read or be sent that needed a person or a redactor? Check INV-1 to INV-4 in `AGENTS.md`.
- Did a `preapproved` rule, a default or a new code path get **less strict**? Defaults must stay on the private, less powerful side.
- Does new outside text (files, command output, git data, web, a new collector) set the taint flag and get escaped and labeled as data?
- Any literal secret in config, code, tests, fixtures, logs, error text or the audit log? Any new log line that could carry a prompt, answer or tool result?
- Anything run through a shell, or with an inherited environment?
- Does a refusal test exist for each new dangerous input? A test that only shows the good path is not enough.
- Are failures fail-closed (deny, ask, refuse), not fail-open?

## 2. Tests

- The test fails without the change. (Revert the product code mentally or actually and see.)
- No wall-clock assertions, no sleeps for ordering, no silent skips (`have`/`have_tool`), no shared global state, no real network or model.
- Placed at the lowest layer that shows the behaviour. Terminal behaviour in the pty or tmux layer, the rest below.
- For `command/`, `tools/`, `validate/`, `engine/tool_calls/`, `present/`, `render.rs`, `sanitize.rs`, `secrets.rs`, `secret_cmd.rs` or the redactor: was a mutation run done (`cargo mut -p ... -f ...`) and were survivors killed or excluded with a reason?
- New tests do not pin incidental wording.
- Does new output go through a `Presenter` (so it is escaped and follows colour settings), and does text that is not ours reach the terminal only through the wrapper? A new escape sequence may come only from `present/theme.rs`.
- Does new core code read the environment, the clock or files through `Host` (`ports.rs`), and do its tests use `MapEnv`/`ManualClock`/`MemFs` instead of `set_var` or the real machine?

## 3. Product

- User-visible change has a `CHANGELOG.md` line under "Unreleased" and the website page is updated (config reference, CLI reference, guide, security model, architecture, roadmap as applicable). Defaults in the docs match the source.
- New config keys: strict parsing, a safe default, an actionable error with a hint, `config check` coverage.
- Errors say what happened and what to do, through `Error::new(code, msg).with_hint(...)`.
- Short internal limits go through `limits::scaled`.
- Start-up stays cheap (about 8 ms, 30 ms budget). A change to start-up, context, config loading or rendering should come with before and after numbers from `tools/bench/perf.py`.
- A prompt change comes with before and after eval results and no new unsafe suggestion (`.agents/skills/change-prompt/SKILL.md`).
- Glue changes behave the same in fish, zsh and bash, were run through `tools/e2e/run-all.sh`, and never override a user's binding.

## 4. Build hygiene

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` are clean.
- No new dependency without a reason. A new one passes `deny.toml` (licenses, advisories, sources; CI runs `cargo-deny`) and does not raise the minimum Rust (1.86) for shipped code. Plugin crates depend on `shellmate-api` only.
- No change to pinned tool versions (`proptest`, `cargo-llvm-cov`, `cargo-mutants`, Node, pnpm, the Rust toolchain) unless that is the purpose of the change.
- `mise.lock` is not changed by accident.
- Commits are small and logical with a message that says why. No secrets, no large generated files, no `docs/` (git-ignored internal notes).

## 5. Claims

Check that the description and the docs do not overstate. Known limits are listed in `AGENTS.md` and the roadmap page. A protection is not "complete" if it is advisory or untested.

## Output

A short list: **Blocking**, **Should fix**, **Nit**, then **Checked and fine** and **Not checked**. Quote the line, say the failing scenario in one sentence, and suggest the fix when it is small.
