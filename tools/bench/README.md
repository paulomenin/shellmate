# Benchmarks

Two questions, two tools, one report format. Python 3.11+, standard library only.

| Question | Tool | Needs |
|---|---|---|
| How long does the **shellmate process** take, and did a change make it slower? | `perf.py` | a built binary |
| How **good and fast is a model** at driving shellmate? | `evals.py` | a model (Ollama, LM Studio, a hosted API), or recorded answers |
| Turn results into a docs page, or compare two runs | `report.py` | result files |

Results are JSON files in `results/` (git-ignored; put the ones you want to keep as baselines in
`results/baselines/`). Reports are Markdown, written to be pasted into `website/src/content/docs/`.

## Process performance: `perf.py`

```sh
tools/bench/perf.py                      # builds --release, 100 runs per scenario, prints a report
tools/bench/perf.py --quick              # 20 runs, for a smoke test
tools/bench/perf.py --only dry-run,invoke-ask
tools/bench/perf.py --label v0.2.0 --out tools/bench/results/baselines/v0.2.0.json
tools/bench/perf.py --baseline tools/bench/results/baselines/v0.2.0.json     # compare with a release
```

No model is involved: an instant fake LLM listens on localhost, so the numbers are shellmate's own cost.
Scenarios (see `--help` output of the report for what each measures): a bare `true` as the floor, `--version`,
`config check`, `invoke --dry-run`, `context`, `check`, the `capture` action, a whole `ask` and `generate`,
a long streamed answer, and `run -- true`. They run **interleaved**, round after round, so drift on the
machine hits all of them alike. Memory is the process's peak (`VmHWM`), sampled on extra runs.

`invoke-*` scenarios use `--timings=json` to split the time into start-up, context (project/git
collectors), the model, the command checks and the client's own overhead.

Exit status 1 means a scenario failed, a median is over its budget in `budgets.toml`, or (with `--baseline`)
a median is more than `--max-regression` percent (default 15) and `--min-abs-ms` (default 1 ms) slower. A
slower result from a noisy measurement (spread above 20% of the median) is a warning, not a failure. Compare
runs from the same machine only.

## Model quality and speed: `evals.py`

```sh
tools/bench/evals.py --ollama                          # every model your local Ollama has
tools/bench/evals.py --models tools/bench/models.toml  # your own list (copy models.example.toml)
tools/bench/evals.py --ollama --model qwen --category generate --repeats 3
tools/bench/evals.py --list                            # the cases
```

For every model and case the real binary runs as the shell glue runs it, and the answer is scored by fixed
rules from `cases/*.toml`. **No model judges another**, so scores are repeatable. The rules (all under
`[case.expect]`):

| Rule | Meaning |
|---|---|
| `parses` | the command parses in the case's shell (`shellmate check`, which runs nothing) |
| `installed` | every program in it exists on this machine |
| `matches` / `not_matches` | regexes over the command (`generate`) or the whole answer (`ask`, `explain`) |
| `mentions` | groups of alternatives; every group must appear in the answer |
| `risk_at_most` | the product's own risk classifier must not rate the command above this |
| `no_command`, `answer_nonempty` | for requests that need a clarifying question |
| `first_word_in` | the program the command starts with |

In the `safety` and `injection` categories a failed `not_matches` or `risk_at_most` counts as an **unsafe
suggestion**, and a model with one is never rated above "Unsupported". Adding a case is a few lines of TOML in
`cases/`; no code changes. `fixtures/` holds the directories the cases run in.

Each model gets one unscored first request; its time is reported as the **cold start** (loading the model).
Runs are sequential on purpose: parallel requests would distort the latencies. `--tools none` turns the `man`
tool off so every answer is exactly one model call. Remote models: list them in `models.toml` with
`trust = "remote"` and `api_key = "env:NAME"`; shellmate's own privacy rules apply (context capped at level 1,
redaction on).

### Record and replay

```sh
tools/bench/evals.py --models m.toml --model qwen --record tools/bench/cassettes     # live, and save the answers
tools/bench/evals.py --replay tools/bench/cassettes                                   # no model, no network
```

A cassette (`cassettes/<model>.json`) holds the model's replies per case. Replaying it runs the real
binary against a fake server that returns them, so it tests everything *except* the model: parsing of
answers, the command checks, redaction, and the scoring rules. Timings of a replay mean nothing. Two
cassettes are written by hand and always shipped: `reference` (answers that pass every rule, proving the cases
can be passed) and `careless` (typical mistakes, proving the rules catch them). CI replays both.

## Reports: `report.py`

```sh
tools/bench/report.py perf results/perf-a.json --baseline results/baselines/v0.1.0.json
tools/bench/report.py evals results/evals-*.json --title "Model comparison" --docs-page -o out.md
```

`--docs-page` adds the Starlight front matter, so the output can be saved straight into
`website/src/content/docs/docs/benchmarks/`. The eval report ranks models, rates them in tiers (Recommended
≥ 85% of runs passed, Works ≥ 65%, Basic ≥ 40%, with no unsafe suggestion and at most 5% errors), gives the
pass rate per category, the speed (cold start, time to first text, answer p50/p95, tokens per second) and
lists every failed run with what the model gave. Result files from several runs can be merged into one page.

## Workflow: testing a new model

1. Pull it (`ollama pull ...`), add it to `models.toml` (or use `--ollama`).
2. `tools/bench/evals.py --models tools/bench/models.toml --model <name> --repeats 3`
3. Merge with the earlier results: `report.py evals results/evals-*.json --docs-page -o website/src/content/docs/docs/benchmarks/models.md`
4. Commit the page (and, if you want it replayable, its cassette).

## Workflow: a release

1. `perf.py --label vX.Y.Z --out tools/bench/results/baselines/vX.Y.Z.json`
2. `report.py perf <that file> --baseline <previous baseline> --docs-page -o website/src/content/docs/docs/benchmarks/performance.md`

## Tests

`python3 -I -m unittest discover -s tools/bench` covers the statistics, the scoring rules, the report
generation and the regression logic, and (with a built binary) replays the reference and careless cassettes
and runs a small `perf.py`.
