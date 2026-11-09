---
name: triage-bug-report
description: Reproduce and diagnose a shellmate bug report without sending anything to a model and without handling the reporter's secrets. Which commands show what happened, what to ask for, and what must never be pasted.
---

# Triage a bug report

shellmate was built to be inspectable without leaking content. Use those tools before guessing.

## What to ask the reporter for

- `shellmate --version`, the OS, the shell and its version, and the prompt framework (starship, powerlevel10k, none).
- The output of `shellmate doctor` (add `--json` if offered) and `shellmate config check`.
- The config with every secret removed. Secrets should only be `env:NAME` or `file:PATH` references anyway. If a literal key appears, tell the reporter to rotate it.
- The exact keys pressed and what they expected to see.
- `shellmate invoke --explain` output for the failing request (one line per fact, no content) and `--timings` for a slowness report.

## What must never be requested or pasted

Prompts, model answers, captured command output, file text read by the tool, API keys, tokens, `.env` contents, or the audit log lines that contain paths the reporter considers private. The audit log has no prompts or answers by design, so it is safe to ask for `shellmate audit show --last 20`, but still ask the reporter to look at it first. If you receive a secret by accident, do not copy it into issues, commits, tests or fixtures. Tell the user.

## Inspect without sending anything

```sh
shellmate doctor                        # installation, keys, shell integration, provider and decider reachability
shellmate config check                  # validates config and secret references; no network
shellmate config show                   # the effective config
shellmate context --action ask          # per provider: which context pieces would be sent and why others are held back
shellmate context --full                # adds the exact redacted payload (still sends nothing)
shellmate check -- 'some command'       # parse, programs exist, risk; runs nothing
printf 'v=1\0action=ask\0shell=bash\0cwd=/tmp\0cursor=0\0buffer=how do I list files\0' | shellmate invoke --dry-run
shellmate audit show                    # what shellmate did, never what was said
```

`invoke --dry-run` prints the request after redaction and exits before contacting a provider. The envelope on stdin is the glue's protocol: NUL-separated `key=value` fields with `buffer` last (see `crates/shellmate/tests/common/mod.rs::envelope`).

## Reproduce with the mock, not the reporter's model

Most bugs are in parsing, rendering, the glue or the checks, and do not need a real model.

- Put the reporter's model reply into a mock: `FakeProvider`, `MockHttpServer` or `MockSseServer` in `tools/shellmate-testing`, or `tools/e2e/mock_openai.py` for the shell suite. Streaming bugs often depend on chunk boundaries; try splitting the reply in the middle of a code fence, a tool block or a multi-byte character.
- Shell and key bugs: reproduce in the real shell with `tools/e2e/conformance.sh <shell>` or by hand in tmux.
- Timing bugs: `invoke --timings=json` splits start-up, context, model, first text, checks and overhead. A slow git collector shows up as context time; large repositories and slow disks are covered by `SHELLMATE_TIME_SCALE`.
- Terminal and approval bugs: see `crates/shellmate/tests/approval_pty.rs` for how to drive a pty.

## Classify

| Kind | What to do |
|---|---|
| A safety rule was bypassed or weakened (a command ran unasked, a secret reached a provider, content in the audit log) | Treat as a security issue. Do not discuss exploit details in a public issue. Follow `.agents/skills/security-sensitive-change/SKILL.md`, write the failing test first |
| Wrong or surprising output from a model | Not a bug in shellmate unless parsing, the prompt framing or a check was at fault. Look at `tools/bench` cases; consider adding one |
| Shell, key or prompt-framework problem | Glue bug. `.agents/skills/add-shell-or-key-binding/SKILL.md` |
| Slow | Measure first (`--timings`, `tools/bench/perf.py`), then fix; add a budget if it regressed |
| Platform not supported yet (macOS, musl, older shells, powerlevel10k) | Documented as untested on the roadmap. Reproduce if you can; otherwise record it |

## Fix

Write a test that fails for the reported reason first, at the lowest layer that can show it. Fix. Run fmt, clippy and the tests. Add a `CHANGELOG.md` line under "Unreleased". Mention the cause, not just the symptom, in the commit message.
