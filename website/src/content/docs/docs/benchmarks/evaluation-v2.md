---
title: Model evaluation v2
description: Compare generators and deciders in disposable containers, measure answer style and cost, and review every answer.
---

The evaluator in `tools/bench/v2` compares main models, checker models, or pairs of both. It runs the real shellmate binary in fresh Docker containers with a controlled package set. The original evaluator and [published model results](../models/) remain available. There are no published live results from v2 yet.

## Inspect the plan

From the repository root:

```sh
python3 tools/bench/v2/evals.py --plan
python3 tools/bench/v2/evals.py --list
```

These commands validate the configuration and cases without Docker, API keys or provider requests. The default example uses placeholder model IDs. Copy `models.example.toml` to `models.toml` in the same directory and replace them with models you have chosen.

The configuration separates providers, main models and deciders. Select `generator`, `decider` or `combined` modes. Deciders can be chat models or native System One models. In decider mode every checker judges the same 28 fixed candidate commands. Combined mode measures generator/checker pairs; the checker remains advisory, just as it is in the application. Composed mode judges the answers a generator gave in an earlier run, replayed from its recording, so a new checker costs only its own requests.

The suite contains 69 main-model cases (24 of them offer tools) and 28 fixed decider cases. Six main-model cases have behavioral assertions. The suite covers command generation, explanations, clarification, shell dialects, safety, injection, answer style and tool calling. Bash has more coverage than fish and zsh. Conversations are not evaluated in this version.

## Build and run

```sh
docker build -f tools/bench/v2/Dockerfile -t shellmate-evals:v2 .
python3 tools/bench/v2/evals.py --config tools/bench/v2/models.toml --out /tmp/model-eval
```

The build downloads compiler dependencies and OS packages, never models. Packages come from a fixed Debian snapshot. Each run records the resolved image ID, installed package versions, binary version, suite hash, harness hash and request hashes. Use image digests for published baselines.

API keys are eval-only references: `env:SHELLMATE_EVAL_API_KEY` or `prompt:EVAL_KEY` for a hidden prompt. A host proxy holds the resolved credentials. Containers receive no API keys. Remote providers still pass through shellmate's redaction chain. Results and recordings scrub known credentials echoed by providers.

The configuration supports repeats, timeouts, a request limit, output-token limit and estimated spending limit. Set prices and their date explicitly. Cached input and reasoning token details are retained when providers return them. Missing costs remain unknown. Missing usage needed for pricing stops subsequent live calls. The cost guard is an estimate; it cannot enforce the provider's final bill.

## Behavioral execution

The normal run does syntax and fixed-rule checks without running suggested commands. With `--behavior`, each behavioral candidate asks for newly typed `yes` at a real terminal. Missing terminals and pasted input deny execution.

Approved commands run in a separate fresh container with no network or provider socket. Containers have a read-only root, an unprivileged user, dropped capabilities, resource limits and a disposable workspace. The host repository, home, devices and Docker socket are not mounted. Containers reduce exposure but do not guarantee protection from container or kernel escapes.

With `--auto-approve`, no one is asked: the command still runs only in the networkless container, and one the product rates high risk (or cannot rate) is not run. Unapproved behavioral checks remain incomplete. They never count as passed. This preserves the application's requirement that a person approve model-generated execution.

## Tool calling

Some cases offer the model shellmate's tools: `man` to read a manual page, `read` to read a file in the working directory, and `exec` to run one simple command. A tool is a text protocol. The model replies with only a block that names the tool, shellmate runs it or refuses it, and the model answers with the result.

The cases check three things: that the call is written the way shellmate reads it, that it is the right tool (or none, when the question needs none), and that the final answer uses what came back. A made-up command, `quokka`, and files with invented contents hold the facts, so a right answer proves the tool was called. Other cases tempt the model to delete files, install a package, read a credential file, or obey a line hidden in a file or in a command's output.

Nothing runs without a person unless the case lists a standing rule, and there is no person in the container, so a call no rule covers is refused. Shellmate refuses the dangerous calls anyway. The score counts what the model asked for, so a model that asks for one is marked unsafe.

Tool cases have their own tier and rates in the "Tool use" table of the [chat models](../chat-v2/) page. They are not part of the chat tier or of the speed figures, and pairs leave them out for now.

## Publish results

Raw reports stay on your machine. `publish.py` adds their runs to a small committed store, one JSON file per model setup, and the results pages are written from that store only. Running the same model again with the same settings, suite and shellmate build adds to its sample; anything else replaces its entry. The [results overview](../results-v2/) explains the tiers, and there is a page each for [chat models](../chat-v2/), [decision models](../decision-v2/) and [composed results](../composed-v2/).

## Review results

Every completed request updates the output directory:

- `results.json` preserves all answers, commands, checks, usage, costs and provenance.
- `summary.md` and `summary.json` compare profiles and categories.
- `review.html` shows every answer with fixture text and expected checks. Add ratings and notes, export them as JSON, and import that file to resume review.
- `cassettes/` records live responses separately for each profile, for offline replay.

`--blind` hides identifying metadata in the HTML review. Answers can still identify their model; the underlying JSON is not blinded. Human ratings stay separate from automated scores.

Answer-style metrics distinguish command-only, prose-only and explanation-plus-command responses. Word counts and command-block counts describe verbosity without treating longer answers as better. Explanation cases can require substantive prose. Decider metrics include coverage, match and risk correctness, match-probability calibration, missed mismatches, false mismatch alarms and missed destructive commands.

Every request, the rules that judge it, and what each failure in the results means are described on the [cases page](../cases-v2/). Scores measure only their explicit rules. Behavioral assertions and human review provide stronger evidence than keyword matches. Repeated cases are not independent evidence of broad safety.

## Offline checks

```sh
python3 -I -m unittest discover -s tools/bench/v2 -v
python3 tools/bench/v2/evals.py --config tools/bench/v2/replay.toml \
  --replay tools/bench/v2/cassettes --out /tmp/eval-v2-replay
```

The unit and broker tests need no Docker or models; the binary integration test needs a built shellmate binary and local socket permissions. The full replay needs the Docker image. Replay refuses missing responses instead of contacting a provider. Its costs and model timings are not live measurements.

For configuration details, recording formats, case authoring and known limits, see the [v2 README](https://github.com/paulomenin/shellmate/tree/main/tools/bench/v2).
