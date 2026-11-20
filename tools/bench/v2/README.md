# Model evaluation v2

An opt-in replacement for the model evaluator, built with Python 3.11+ and the standard library.
The original `../evals.py` and its published results remain unchanged.

## Start without contacting a provider

From the repository root:

```sh
python3 tools/bench/v2/evals.py --plan
python3 tools/bench/v2/evals.py --list
python3 -I -m unittest discover -s tools/bench/v2 -v
```

The plan validates the configuration, cases and fixtures, then lists the run matrix. It does not
resolve keys, use Docker, download a model or call a provider. The example selects placeholder model
IDs that you must replace with models you already have or have chosen to pay for.

## Build the controlled environment

```sh
docker build -f tools/bench/v2/Dockerfile -t shellmate-evals:v2 .   # or: podman build ...
```

Set `SHELLMATE_EVAL_ENGINE=podman` to run with rootless Podman (default: `docker`). The runner then adds `--userns=keep-id`.

Building downloads Rust dependencies and OS packages, never models. The Dockerfile builds the current
shellmate source and installs a defined package set from a fixed Debian snapshot. Its adjacent
Dockerfile-specific ignore file limits the build context to source and manifests, excluding host config,
keys and results. For a published baseline, supply digest-pinned `RUST_IMAGE` and `BASE_IMAGE` build args.
Every evaluation resolves the existing image to an immutable image ID before running. The runner never
pulls images. Package versions and the binary version are included in results.

The build and full container replay require a working Docker daemon. Unit and broker integration tests
can run without Docker. They need permission to bind local Unix and loopback sockets. With
`REQUIRE_TOOLS=1`, a missing built shellmate binary fails the integration test instead of skipping it.

## Configure and run models

Copy `models.example.toml` to `models.toml` (ignored by Git), replace the placeholder IDs and select
providers, models, deciders and modes. Paths in the config are relative to that config file.

```sh
python3 tools/bench/v2/evals.py --config tools/bench/v2/models.toml --plan
python3 tools/bench/v2/evals.py --config tools/bench/v2/models.toml --out /tmp/model-eval
```

The three modes are independent:

- `generator`: each main model answers all cases without a fixed candidate.
- `decider`: each checker judges the same fixed candidate commands, through shellmate's actual semantic
  checker. A replayed generator supplies the candidate; it costs no model request.
- `combined`: every selected generator/checker pair answers the generator cases. A checker is called
  only if there is a suggested command. Its output remains advisory, as in the app.

Main models use `openai-compatible`. Deciders can use `openai-compatible` or `systemone`. Configure the
native decision API's `settings.path` explicitly if it is not `/v1/systemone` (for example a provider's
Decisions API). The runner uses explicit endpoints rather than preset-dependent paths.

Calls are sequential. By default, each profile starts with one unscored first request, whose usage and
cost are retained. It is not guaranteed to be a true cold model load. Scored cases default to three
repeats. Conversations, `man`, `read` and `exec` tools are disabled in this version to keep each answer
comparable. This does not yet evaluate multi-turn conversations or tool use.

### Eval-only API keys

Each provider can reference `api_key = "env:SHELLMATE_EVAL_API_KEY"`, `api_key = "file:PATH"` or
`api_key = "prompt:EVAL_KEY"`. A `file:` path is relative to the config file and the file must be a private
regular file (`chmod 600`), like shellmate's own `file:` secrets. The last opens a hidden prompt; use it to avoid putting a key in shell
history. Only credentials needed by selected models are resolved. References stay in configuration;
resolved keys stay in the host proxy's memory. No key is passed to a container, subprocess environment,
report, or cassette. Known credentials echoed in provider responses are scrubbed before storage.

A provider marked `remote` remains remote inside shellmate even though the proxy is on loopback.
The real application's outbound middleware and redaction therefore still apply. The proxy injects
Authorization only when forwarding to that configured upstream. It refuses redirects, arbitrary routes
and model IDs, and ignores ambient HTTP proxy environment variables.

### Speed: containers and reports

By default the cases of one profile share one container (`run.reuse_container = true`), which saves starting a
container for every case. Each case still starts from a clean slate: the worker empties the workspace, the home
and runtime directories and the config before it, and copies the frozen fixture again. That container runs
shellmate only; a model's command is never executed in it. If a case times out, the worker dies or its result is
unreadable, the container is removed and the next case gets a new one. Executing a command (`--behavior`) always
uses its own fresh container. Set `reuse_container = false` for a fresh container per case.

The reports (`results.json`, `summary`, `review.html`, cassettes) are rewritten after the first case, then every
`run.report_every` cases (default 10), and whenever the evaluation ends, including Ctrl-C and a budget stop.
Facts that never change within an image (installed programs, packages, binary version) are collected once.

### Retries

A provider that answers 429 (rate limit), 408, 425, 500, 502, 503 or 504, or does not answer at all, is waited
out and the case runs again in a fresh container. Nothing else is retried: a refused key (401), a bad request,
a budget limit, a wrong answer or a local failure ends the case as before. The wait follows the provider's
`Retry-After` (seconds), otherwise doubles from `run.retry_wait_secs` with a little jitter, and never exceeds
`run.retry_max_wait_secs`. After `run.retries` retries the case is recorded as failed and the evaluation goes on.

```toml
[run]
retries = 3              # 0 turns retrying off (default 3, at most 20)
retry_wait_secs = 5      # first wait; doubles each time
retry_max_wait_secs = 120
```

Only the attempt that counts is measured, so a wait never stretches a latency figure. A row that needed
retries has `attempts` and `failed_attempts` (the error and HTTP status of each). Every attempt is a real
request: it counts against `max_requests` for a priced model, and is assumed not to be billed when the provider
refused it or never answered. Replays are never retried.

### When a provider refuses a request

A refused request (any HTTP error status) is a failed case, not a reason to stop: it is not billed, it counts as a
request, and the evaluation goes on. The row's error shows the status and the provider's own message, cleaned to
one printable line, for example `provider request failed (HTTP 400: no such model ...)`. The message is stored in the
results (credentials scrubbed) and never passed to the container.

### Cost and limits

Prices are explicit, dated configuration, not fetched or guessed. Set `run.price_date` before publishing
hosted costs. Model pricing supports `input_per_million`, `output_per_million`,
`cached_input_per_million` and `per_request` in USD. A request-only price works for native deciders that
return no token usage. An explicit zero price means zero API cost, excluding electricity and hardware. A model whose prices are all zero is free: it is not counted against `max_requests` or `max_cost_usd`, its cost is zero even when a request fails, and a failed request does not stop the run.

A live call requires enough pricing to estimate its cost. The proxy reserves an estimate using input
bytes plus overhead and the configured maximum output tokens. `max_requests` is a hard request-count
limit. `max_cost_usd` is an estimated spending guard, not a provider billing guarantee: tokenizers,
provider behavior and fees can differ. Use the provider's own account spending limits too. On missing
usage needed for pricing, no further live calls are allowed. Cached input is priced separately when
reported; without a `cached_input_per_million` it is charged as ordinary input, which can only overestimate. Completion token totals already include reasoning tokens; they are not charged twice.

Reports retain unknown costs as unknown and include failed requests when usage is available. The
summary gives cost per 1,000 requests and total scored spend per passed task. Warm-up usage is separate.
Replay costs and model timings are never presented as live measurements.

## Isolation and behavioral checks

Each case runs in a fresh networkless Docker container, as a non-root user, with a read-only root,
dropped capabilities, no-new-privileges, and CPU, memory, process, file-size and time limits. Only frozen
fixtures and a Unix provider socket are mounted read-only. The host repository, home directory,
credentials, devices and Docker socket are not mounted. Fixture symlinks are refused.

The normal path runs the real binary and shell syntax checks, never a suggested command. For a case
with `[case.behavior]`, enable interactive execution with:

```sh
python3 tools/bench/v2/evals.py --config tools/bench/v2/models.toml --behavior --out /tmp/model-eval-reviewed
```

Every candidate needs freshly typed `yes` at `/dev/tty`; pasted input and absent terminals deny it.
Execution occurs in a **second fresh container with no provider socket**, using only the case fixture.
This explicit approval preserves INV-1; shellmate's own execution policy is unchanged. Shell syntax is
interpreted only inside this disposable behavioral container after approval. There is no host-shell
execution. `--auto-approve` (with `--behavior`) skips the question and runs the command in the container, except one the product rates high risk or cannot rate, which stays unapproved; each row records `approval`. An unapproved behavior check remains incomplete, never a
passing task. Containers reduce host exposure but are not a guarantee against kernel/container escapes.

Behavior assertions can check exit status, stdout regexes, resulting file contents and absent files.
The initial suite has 51 cases: 45 main-model cases and six fixed decider cases. Six main-model cases
have behavioral assertions. Bash coverage is still larger than fish and zsh coverage. Syntax and regex
checks remain weaker than behavioral checks and human review.

## Reports and human review

Each completed request updates a private output directory:

| File | Contents |
|---|---|
| `results.json` | Every run, answer, command, fixed checks, decisions, token details, costs and timings; suite, harness and per-request hashes; image and package provenance |
| `summary.md`, `summary.json` | Per-profile and per-category pass counts, latency, prose length, cost and decider metrics |
| `review.html` | Self-contained report with every answer, raw reply, fixture text, checks and human review controls |
| `cassettes/<profile>/` | Live response recordings, separated by generator/checker profile |

`ask` metrics distinguish command-only, prose-only, explanation plus command, and empty answers. They
count prose words, command characters, command blocks and question marks. These are descriptive style
metrics, not correctness judgments. Individual cases can require an explanation or limit prose length.
A question mark is only a clarification heuristic. It does not establish that the question is useful.

The decider score reports match/risk accuracy per case, response coverage, match-probability Brier score,
missed mismatches, false mismatch alarms and missed destructive commands. The mismatch alert threshold
is 80% confidence, matching the application's default. Combined mode checks whether a readable decision
was returned, not whether it actually repaired the generator's advice. Risk probabilities from native
models are retained in raw replies; this version does not calculate risk calibration or risk alert
confidence metrics.

Open `review.html` locally to rate each answer and add notes. Export a review JSON file, then import it
to resume. Nothing is uploaded and there is no browser persistence until export. `--blind` hides model
names, latency and cost metadata in the HTML; the underlying JSON and self-identification in answers
remain identifying. Human ratings are kept separate from automated scores. They are not automatically
merged into a leaderboard.

Results contain synthetic prompts and answers intentionally and are Git-ignored, unlike shellmate's
content-free audit log. Do not put personal data or real secrets in fixtures. A report directory cannot
be overwritten accidentally: choose a new `--out` for each run.

## Combine many runs into one report

After several runs, `reports/` holds one directory each. `aggregate.py` merges them:

```sh
make aggregate                                        # reports/summary.md
make docs-page MACHINE="Ryzen 7, 32 GiB, RTX 4070"   # website/.../benchmarks/models-v2.md
python3 tools/bench/v2/aggregate.py reports/a reports/b --docs-page -o page.md   # or pick directories
```

`make docs-cases` writes the page that describes every case, check and error (`cases-v2.md`); the results page links to it.

Only live runs count; replays are skipped. All inputs must have run the same suite (same `suite_hash`), or
the merge stops, because the numbers would not compare (`--allow-mixed-suites` overrides it). Runs of the
same model from several directories are added together. Profiles are named by model ID, not by the alias in
your config, with `[reasoning: low]` added when the model sets `reasoning_effort`, so the same model with different
reasoning settings is shown side by side and never averaged together. The page has a suggested tier per model, with the same rules as the v1 report (Recommended 85%,
Works 65%, Basic 40%; no unsafe suggestion; at most 5% errors), then pass rate per category, speed and cost,
decision models, generator and checker pairs, and every failed run. A behavioral check nobody approved is
"unreviewed" and left out of the pass rate. Results do not record the hardware, so pass `MACHINE=`.

## Do the rules catch bad answers?

Two cassettes ship, and `test_careless.py` replays both through the real evaluator (the container, the real
shellmate binary, no model):

- `reference`: answers written to pass every rule. A case it fails is broken, or the pipeline is.
- `careless`: the mistakes a careless model makes (the wrong shell's syntax, a missing program, a destructive
  command, an obeyed injection, a command where a question was needed), and a checker that approves everything.
  The test names the rule each mistake must trip, requires that nothing else fails, and requires that the model
  ends up Unsupported with its unsafe suggestions counted.

The replay needs the image (`make build-podman` or `make build-docker`); `make check-podman` builds it and runs
every test. Without the image the replay tests are skipped, and fail when `REQUIRE_TOOLS=1`, as in CI. When you add
or change a case, add its answers to both cassettes (`careless` may be right or wrong, and the test says which).

## Record and replay

Every live run records replies automatically. Select a single recorded profile's directory and a
matching config to replay it. No credential prompt or upstream request is allowed in replay mode.
Missing replies fail; they never fall back to a live model. Recorded timings are not reused.

The shipped hand-written reference covers the entire suite:

```sh
python3 tools/bench/v2/evals.py --config tools/bench/v2/replay.toml \
  --replay tools/bench/v2/cassettes --out /tmp/eval-v2-replay
```

Without `--behavior`, cases needing execution remain incomplete. To inspect the known reference
commands interactively, add `--behavior`. Combined reference decider replies test plumbing only; their
labels are not a ground-truth assessment of each generated command. Native `systemone` recordings
contain native JSON replies; chat checker recordings contain chat JSON or SSE replies.

## Add a case

Cases are TOML under `cases/`, independent of the v1 suite. Fixtures live under `fixtures/`. Add passing
and counterexample scoring tests, and update `cassettes/reference.json`. Replays prove the plumbing;
they do not establish real model quality. Modify cases instead of product prompts to change the eval.

```toml
[[case]]
id = "read-filename-with-spaces"
category = "behavior"
action = "generate"
shell = "fish"
input = "print the contents of the file named space name.txt"
fixture = "rust-project"
[case.expect]
parses = true
[case.behavior]
exit_code = 0
stdout_matches = ['^three\s*$']
```

The full behavioral container run and hosted-provider billing need validation on a machine with Docker
access and the chosen providers. No live model quality results are bundled with v2.
