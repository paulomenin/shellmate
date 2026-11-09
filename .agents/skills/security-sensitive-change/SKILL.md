---
name: security-sensitive-change
description: Checklist for changes that touch shellmate's safety guarantees: the command classifier, approval prompt, exec/read/man tools, redaction, taint, context levels, config secrets, or the audit log.
---

# Security-sensitive change

shellmate does not trust the model. The model can be wrong, manipulated by text it reads, or hallucinate. These rules are enforced in code, not hoped for. Public description: `website/src/content/docs/docs/concepts/security-model.md`.

## Where the code is

| Concern | Where |
|---|---|
| Classifier, strict command parser, rules, risk | `crates/shellmate-core/src/command/` |
| Tool parsing, policy, running `man`/`exec`/`read`, sanitized env, caps | `crates/shellmate-core/src/tools/` |
| The decision for each `exec`/`read` call (parse, classifier, pre-approval, taint, ask the person), as `decide` functions that run nothing | `crates/shellmate-core/src/engine/tool_calls/` |
| The approval prompt, typeahead flush, no-tty deny | `crates/shellmate/src/approval.rs`, `tty.rs` |
| Redaction | `crates/plugins/middleware-redact/src/` |
| Taint, context levels, outbound chain, INV-2 check | `crates/shellmate-core/src/engine/`, `collect/`, `capture.rs` |
| Command validator (notes only, never blocks) | `crates/shellmate-core/src/validate/` |
| Config, secrets as `env:`/`file:`/`cmd:` | `crates/shellmate-core/src/config/`, `secrets.rs`, `secret_cmd.rs` (runs `cmd:` programs), `engine/lazy.rs` (fetches them on first use) |
| Audit log | `crates/shellmate-core/src/audit.rs` |
| Escaping untrusted text for the terminal, and the only place escape sequences are written | `crates/shellmate-core/src/sanitize.rs`, `render.rs`, `present/` (the theme writes the styles; everything else goes through the wrapper) |

## Rules that do not bend

1. **Nothing the model produces runs without a person** (INV-1). A suggestion goes to the input line. An `exec`/`read` call needs a key at a prompt on the terminal. Only a single key counts, earlier typeahead is flushed, no terminal means deny. High risk needs the word `yes`.
2. **The classifier keeps its veto.** A `preapproved` rule never makes a flagged command run unasked. When unsure, the classifier rates higher. Conservative false positives are acceptable; a missed dangerous command is not.
3. **Commands are never run through a shell.** One simple command, argv, sanitized environment (no secrets), timeout, capped output, process group killed on timeout. The strict parser accepts a line only if it contains no shell syntax. The sanitized environment is an allow list copied from the `Env` given to `run_exec`; `ExecEnv::Inherit` is the real process environment on purpose.
4. **Remote means redacted** (INV-2). Redaction fails closed: if it cannot run, the request does not go out.
5. **Untrusted content is data** (INV-4). Anything a program or another party wrote is escaped, labeled as data, and sets the taint flag, which suspends pre-approvals for that request. New sources of outside text must set it.
6. **Credentials are never readable by tools.** `.env`, `.ssh`, `*.pem`, `.git/config` and similar are refused and no rule unlocks them. A missing path and an outside path get the same answer. Symlinks are resolved before the check.
7. **Secrets live only in references** (`env:`, `file:`, `cmd:`). A `cmd:` line is split by a strict splitter with no shell syntax, runs directly with a small environment, no stdin, a timeout and a size cap, and is read only from the user's own config. Errors name the program, never its arguments or output. Never as literals in config, tests, fixtures, logs, errors or the audit log.
8. **The audit log has no content**: no prompts, answers or tool results. A test checks this.
9. **git runs with `core.fsmonitor` disabled**, so a cloned repository cannot execute code.
10. **Cleartext HTTP only for loopback** endpoints (`check_endpoint`).

## Before you change anything

- State which invariant is involved. If a change makes any of the rules above weaker, stop and ask the user. Do not "fix" a failing safety test by loosening it.
- Look for the existing tests of the behaviour (`grep` the function name in `tests/` and the in-file test modules). Read them first, they document intent.

## While you change it

- Fail closed. On parse failure, unknown input or an unexpected state, the answer is "ask" or "deny", never "allow".
- Add the test that shows the **dangerous input is refused**, written first if you can, and watch it fail for the right reason. For `exec`/`read` write it against `decide` (`engine/tests/tool_decisions.rs`), where nothing runs; tests get their environment, files and clock from `Host` (`MapEnv`, `MemFs`, `ManualClock`), never from `set_var`.
- Property tests exist for the classifier (no panic, only simple lines are Safe, destructive programs stay Unsafe however quoted or chained, glob matching) and the redactor (no leak, round trip, idempotence, stable placeholders, chunk-invariant streaming). Extend them when you add a new shape of input. If a property fails, find out whether the product or the property is wrong.
- New redaction pattern: add the secret shape to the known-shapes test and a property case. Check that restoring does not re-redact placeholders or safe words.
- New tool or new tool argument: it needs a parser that rejects everything not explicitly allowed, a classifier rule, a policy decision, the approval text that shows the exact command, and the taint behaviour for its output.

## After you change it

```sh
cargo test --workspace --locked
cargo mut -p shellmate-core -f '<the files you changed, as a glob>'     # or: mise run mutants
```

Kill each surviving mutant with a test or exclude it with a written reason (`.cargo/mutants.toml`). Run `tools/e2e/run-all.sh` if you touched key handling or the prompt. Update `concepts/security-model.md` if a threat row or an invariant statement changes, and add a line to `CHANGELOG.md` under "Unreleased".

## Report

In your summary, say which invariant the change relates to, which test proves the refusal, and what you did not verify. Do not describe a protection as complete when it is advisory or untested (for example the 120 s approval timeout has no test, and the pty tests are Linux-only).
