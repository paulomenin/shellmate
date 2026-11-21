# Benchmarks

Two questions, two tools. Python 3.11+, standard library only.

| Question | Tool | Needs |
|---|---|---|
| How long does the **shellmate process** take, and did a change make it slower? | `perf.py` (here) | a built binary |
| How **good and fast is a model** at driving shellmate? | [`evals/`](evals/README.md) | Docker or Podman, and a model or recorded answers |

`report.py` turns a performance result into a docs page or compares two runs. Performance results are JSON files
in `results/` (git-ignored; put the ones you want to keep as baselines in `results/baselines/`). Reports are
Markdown, written to be pasted into `website/src/content/docs/`.

The model evaluator runs the real binary in disposable containers, scores answers by fixed rules, and keeps its
published results in `evals/published/`. Start with `python3 tools/bench/evals/evals.py --plan` from the
repository root, and see its README for the rest.

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

## Reports: `report.py`

```sh
tools/bench/report.py perf results/perf-a.json --baseline results/baselines/v0.1.0.json
```

`--docs-page` adds the Starlight front matter, so the output can be saved straight into
`website/src/content/docs/docs/benchmarks/`.

## Workflow: a release

1. `perf.py --label vX.Y.Z --out tools/bench/results/baselines/vX.Y.Z.json`
2. `report.py perf <that file> --baseline <previous baseline> --docs-page -o website/src/content/docs/docs/benchmarks/performance.md`

## Tests

`python3 -m unittest discover -s tools/bench` covers the statistics, the regression logic and the report
generation, and (with a built binary) runs a small `perf.py`. The evaluator has its own tests, under `evals/`.
