# Development tools

Nothing here ships in the `shellmate` binary.

| Path | What |
|---|---|
| `shellmate-testing/` | Rust crate with mock HTTP/SSE servers, a fake provider and an in-memory conversation store, used by the tests |
| `e2e/` | Conformance suite: real fish, zsh and bash in tmux against a mock provider (`e2e/run-all.sh`) |
| `bench/` | Benchmarks: process performance (`perf.py`) and model quality and speed (`evals.py`), with Markdown reports for the docs. See `bench/README.md` |
| `mutants.sh` | Mutation testing of the security modules (`mise run mutants`) |
| `spikes/` | Archived experiments from the design phase. Kept for reference, not built or maintained |

## Coverage

`mise run coverage` or `cargo cov` (HTML only; the mise task also prints the summary; needs `cargo install --locked cargo-llvm-cov --version 0.9.1`) runs the workspace tests with instrumentation, prints a per-file summary and writes an HTML report to `target/llvm-cov/html`. The integration tests that spawn the `shellmate` binary count too. The fish/zsh/bash e2e suite does not. `.cargo/config.toml` sets `SHELLMATE_TIME_SCALE=5` (unless you already set it) because instrumented binaries are slow; it never shortens a limit, so plain `cargo test` is unaffected. CI uploads `lcov.info` and prints the totals in the job summary; it reports only and gates nothing.

## Mutation testing

Line coverage says a line ran; mutation testing says a test would notice if it were wrong. `cargo-mutants` changes the code (flips a `>`, empties a function, deletes a `match` arm) and runs the tests. A mutant that no test fails on has *survived*, and that is a gap.

```
mise install                                            # installs cargo-mutants at the pinned version
mise run mutants                                        # command/, tools/, validate/, the engine tool decisions, the presenter and the redactor, about 30 minutes
tools/mutants.sh -p shellmate-core -f 'crates/shellmate-core/src/command/*.rs'   # one file
tools/mutants.sh --list                                 # what would be tried
cargo mut -p shellmate-core -f 'crates/shellmate-core/src/command/*.rs'   # the same through a cargo alias
```

Results land in `target/mutants-*/mutants.out/`; `missed.txt` is the list to work through. For each one either add a test that fails on it, or, when the change cannot be observed, add it to `exclude_re` in `.cargo/mutants.toml` with the reason. A mutant that hangs (a `timeout`) counts as caught. Run it before a release or after touching `command/`, `tools/`, `validate/`, `present/`, `render.rs`, `sanitize.rs` or the redactor; it is too slow for every pull request.
