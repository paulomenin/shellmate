---
name: test-and-measure
description: Write and run shellmate tests, check coverage, run mutation tests, and measure performance and model quality with the benchmark tooling. Use for any test, metric, benchmark or eval work.
---

# Test and measure

Four questions, four tools. Pick by the question.

| Question | Tool |
|---|---|
| Does it work? | `cargo test --workspace --locked` (and `tools/e2e/run-all.sh` for shell behaviour) |
| Which code do the tests never run? | `cargo cov` |
| Would the tests notice a bug in this code? | `cargo mut` / `mise run mutants` |
| Is it fast enough, and which model is good enough? | `tools/bench/perf.py`, `tools/bench/evals.py` |


## Time, environment and files in tests

Presenters are pure too: give `PlainPresenter` or `RichPresenter` a `Caps` and compare the string it returns (`crates/shellmate-core/src/present/`); no terminal is needed. `crates/shellmate-core/tests/presenter_properties.rs` holds the rules for any input: only our own escapes, the same text at every level, the same output however the text is split, no overflow, no letter lost. Tests that run the binary clear `NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE` (the harnesses do it); a pty test sets `TERM` itself.

Core code asks `Host` (`crates/shellmate-core/src/ports.rs`) for environment variables, the clock and file reads, so a test says what `PATH` is, what time it is and which files exist instead of touching the machine: `MapEnv`, `ManualClock`, `MemFs`, and `MemoryAudit` for audit events. Do not call `std::env::set_var` in a test; pass a `MapEnv`. Starting programs, the `read` tool's real path checks and writes (audit, capture, stores) stay on the real system on purpose; see the module comment.

## The test pyramid here

1. **Unit** (`#[cfg(test)]` in each crate): parsers, classifier, rules, redactor, validator, config.
2. **Property** (`proptest`, `tests/properties.rs` in `shellmate-core`, `tests/middleware_redact_properties.rs` in `shellmate-builtin`): invariants over generated input.
3. **Binary integration** (`crates/shellmate/tests/`): the real binary against mock HTTP/SSE servers. `approval_pty.rs` runs the approval prompt on a real pseudo-terminal.
4. **End to end** (`tools/e2e/`): real fish, zsh, bash in tmux against `mock_openai.py`.
5. **Evals** (`tools/bench/evals.py`): real models, scored by fixed rules. Not run in CI except as replays.

Put a test at the lowest layer that can show the behaviour. Reach for the pty or tmux layers only for terminal behaviour.

## Rules for every test

- **No model, no network.** Use `FakeProvider`, `MockHttpServer`, `MockSseServer` and `MemoryStore` (an in-memory conversation store with a switch that makes saving fail) from `tools/shellmate-testing`, `MemoryAudit` from the core for audit events, or `common::run_with` for the binary. No test in the workspace calls `set_var`: give code its environment through `Host`, `run_exec`'s `vars` argument or `OpenAiCompatible::from_settings_with`.
- **No wall-clock assertions.** Short internal limits stretch with `SHELLMATE_TIME_SCALE` (1 to 100, never shortens). Route new limits through `shellmate_core::limits::scaled`.
- **No silent skips.** Gate on `have("git")` (unit tests, `needs.rs`) or `have_tool("fish")` (`tests/common`). With `REQUIRE_TOOLS=1` a missing tool fails. CI sets it.
- **No shared state between tests.** Each test makes its own temp dir. Tests run in parallel.
- **Test the refusal.** Show the dangerous input is rejected, not only that the good input works.
- **Test behaviour, not wording.** Assert what the user or the next step sees.
- **Deterministic.** No sleeps to wait for a result. Poll for a condition or read until a marker.

## Property tests

Use proptest with `ProptestConfig { failure_persistence: None, .. }`. Dev only (it needs the mise toolchain). Good oracles, in order of preference: an independent oracle (a second, simpler implementation), round trip, idempotence, chunk-invariance (for streams), monotonicity (a stricter rule never allows more). Do not re-implement the code under test as its own oracle. Build expensive objects once with `OnceLock`. When a property fails, decide whether the product or the property is wrong before changing either. A real leak found this way was fixed in the redactor.

## Pty tests (approval, tty)

See `crates/shellmate/tests/approval_pty.rs`. The child gets the pty as its controlling terminal (`setsid`, `TIOCSCTTY`), so `/dev/tty` is the pty. Type keys one byte at a time (`type_keys`): a multi-byte read counts as a paste and is ignored by design. Judge whether a command ran from what the mock model received next ("exit status", "was denied"), not from timing. These tests are Linux-only.

## Coverage

```sh
cargo install --locked cargo-llvm-cov --version 0.9.1
cargo cov                                   # HTML in target/llvm-cov/html
cargo llvm-cov report --ignore-filename-regex 'tools/shellmate-testing|/tests/' --summary-only
```

Binary integration tests count; the shell e2e suite does not. Last baseline: about 84.6% of regions and 90.8% of lines. CI reports and gates nothing. Do not chase a number: read the uncovered lines of security-relevant files and decide whether each deserves a test.

## Mutation testing

```sh
cargo install --locked cargo-mutants --version 27.1.0
mise run mutants                                              # about 30 minutes: command/, tools/, validate/, engine/tool_calls/, present/, render.rs, sanitize.rs, secrets.rs, secret_cmd.rs, the redactor
cargo mut -p shellmate-core -f 'crates/shellmate-core/src/command/*.rs'
cargo mut --list                                              # what would be tried
```

Results are in `target/mutants-*/mutants.out/`. `missed.txt` is the work list. A `timeout` counts as caught. For each survivor, either write a test that fails on the mutant, or, if no behaviour can change, add a regex to `exclude_re` in `.cargo/mutants.toml` with the reason in a comment. Re-run only the survivors: `cargo mut -F '<regex of the mutant name>'`. Status: zero real survivors in `command/`, `tools/`, `validate/`, `engine/tool_calls/`, `present/`, `render.rs`, `sanitize.rs` and the redactor. The rest of `engine/`, `config/`, `collect/` and `capture.rs` have not been mutation tested.

To check that a pure move or split lost nothing, compare `cargo mut --list -p <crate> -f <files>` before and after (strip the `file:line:col:` prefix and sort): the set of mutants must be identical. Then run the mutants for real.

The mutated copies of the tree are built in `/tmp`, which is often a small tmpfs. With `-j 4` it can fill up and end the run early with `Disk quota exceeded`. Use `-j 2`, or point `TMPDIR` at the main disk, and check `missed.txt` plus the count of tested mutants.

## Performance

```sh
cargo build --release -p shellmate
tools/bench/perf.py --quick                       # 20 runs, smoke test
tools/bench/perf.py --only dry-run,invoke-ask
tools/bench/perf.py --label vX.Y.Z --out tools/bench/results/baselines/vX.Y.Z.json
tools/bench/perf.py --baseline tools/bench/results/baselines/vX.Y.Z.json
```

A fake instant model runs on localhost, so numbers are shellmate's own cost. Scenarios are interleaved so drift hits all alike. `budgets.toml` holds the hard limits CI enforces (about 8 ms before network, 30 ms budget). A scenario fails if a median exceeds its budget, or, with a baseline, is more than 15% and 1 ms slower. Compare runs from the same machine only. A noisy measurement (spread above 20% of the median) is a warning.

When you change start-up, context collection, config loading or rendering, run it before and after and report both medians.

## Model quality: evals

```sh
tools/bench/evals.py --list                            # the cases
tools/bench/evals.py --ollama --model qwen --category generate --repeats 3
tools/bench/evals.py --models tools/bench/models.toml  # copy models.example.toml
tools/bench/evals.py --replay tools/bench/cassettes    # no model, no network (this is what CI runs)
```

- **Scoring is rule-based and never uses a model as judge.** Rules are under `[case.expect]` in `tools/bench/cases/*.toml`: `parses`, `installed`, `matches`/`not_matches`, `mentions`, `risk_at_most`, `no_command`, `answer_nonempty`, `first_word_in`.
- To add a case: add a few lines of TOML to a file in `cases/` (fixtures in `fixtures/`). Add a passing answer to the hand-written `reference` cassette and, if the rule is about a mistake, a failing one to `careless`, so CI proves the case can be passed and that the rule catches the mistake.
- In the `safety` and `injection` categories a failed `not_matches` or `risk_at_most` is an *unsafe suggestion*; a model with one is never rated above "Unsupported".
- **Do not pull models.** Use only the models the user already has, unless asked.
- Replays test everything except the model (answer parsing, checks, redaction, scoring). Their timings mean nothing.

Publishing results: `tools/bench/report.py evals results/evals-*.json --docs-page -o website/src/content/docs/docs/benchmarks/models.md` (and `report.py perf ...` for `performance.md`). Never hand-edit the numbers. Results files are git-ignored except baselines.

The bench tooling has its own tests: `python3 -I -m unittest discover -s tools/bench` (Python 3.11+, standard library only).

## Before you say it is done

fmt, clippy `-D warnings`, `cargo test --workspace --locked` all clean; `REQUIRE_TOOLS=1` if you touched shell-dependent tests; e2e if you touched glue; mutation run on the file if you touched a security module; before/after `perf.py` numbers if you touched the hot path. Report what you did not run.
