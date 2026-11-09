---
name: investigate-flaky-test
description: Diagnose a test that fails intermittently or only under load, in CI, or under coverage in the shellmate repo. Known causes and how to reproduce them before changing anything.
---

# Investigate a flaky test

Do not add a retry, a longer sleep, or `#[ignore]`. A flaky test means a race, a clock, a shared resource or an unstated dependency. Find which one.

## Known causes in this repository

| Symptom | Cause | Fix pattern |
|---|---|---|
| Passes alone, fails in a full run or under `cargo cov` | A short internal limit (git 600 ms, syntax check 2 s, `man` 5 s) is too tight for a slow or instrumented binary | Limits go through `limits::scaled`. `SHELLMATE_TIME_SCALE` (1 to 100) stretches them. `.cargo/config.toml` sets 5. Do not shorten a limit in a test |
| Passes locally, skipped or failing in CI | A needed program (git, fish, zsh, bash) is missing, and the test skipped silently | Gate with `have(...)` (`needs.rs`) or `have_tool(...)` (`tests/common`). With `REQUIRE_TOOLS=1` missing tools fail. CI sets it |
| `ETXTBSY` ("text file busy") running a script just written | Executing a file another thread still has open for writing | Run it through `sh script` (see `exec_script`, `run_man_with`) instead of executing it directly |
| pty test: a keystroke ignored | A multi-byte `read` is treated as a paste by design | Send one byte at a time (`type_keys`) |
| pty or tmux test: sometimes the screen is not ready | Fixed sleep instead of waiting for a marker | Poll the transcript or pane until a marker appears, with a timeout |
| Seen once, not reproduced: `tools/e2e/conformance.sh fish plain` failed 3 of the 29 `conversation` checks, then passed 6 runs in a row | Probably a fixed `sleep 1` before reading the screen (around lines 187 to 191), with the previous command line still being typed | Replace the sleep with a poll for the expected marker, as in the row above. If it recurs, capture the screen text of the failing run first |
| Two tests interfere | Shared temp path, env var or port | Unique temp dir per test (`std::process::id()` plus a counter), ephemeral ports, no global env mutation in parallel tests: pass a `MapEnv` through `Host` (no test in the workspace calls `set_var` any more; keep it that way). `run_exec` takes the `Env` its sanitized child is built from, and the provider has `from_settings_with` for its key variable |
| Different results on repeat runs of a property test | A genuine counterexample found by chance | Do not discard it. Reproduce with the printed seed or inputs, then decide if the product or the property is wrong |

## Reproduce first

```sh
cargo test --workspace --locked -- --test-threads=1          # ordering and shared state
RUST_TEST_THREADS=2 cargo test --workspace --locked          # less load
for i in $(seq 1 30); do cargo test -p shellmate --test approval_pty --locked -q || break; done   # repeat one test file
cargo cov                                                    # instrumented slowness
nice -n 19 stress-ng --cpu "$(nproc)" --timeout 60s &        # artificial load, if stress-ng is installed
```

Run the single failing test many times under load. If it never fails, read the test for clocks, sleeps and shared paths, and read the code for threads and timeouts.

## Fix

1. Name the cause in the commit message.
2. Remove the race or the clock dependency. Do not widen a tolerance unless the tolerance is the thing under test.
3. Make the test fail reliably once, by forcing the bad ordering, before you fix it, so you know the fix works.
4. If a test legitimately needs time, scale it with `SHELLMATE_TIME_SCALE`.
5. If the product has a real race (for example in conversation locking), that is a product bug. Fix it and add a test.

Report what you ran and how many repetitions passed. "Ran it once and it passed" is not evidence.
