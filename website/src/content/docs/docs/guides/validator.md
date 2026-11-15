---
title: Checking suggested commands
description: Notes under a suggested command, and optional decision models that judge whether it matches your request.
---

Before a suggested command reaches your input line, shellmate checks it. The checks only add notes under the answer. **The command is always still placed in your input line**, and you decide.

```text
$ list python files bigger than 1MB            ← alt+i
$ find . -name '*.py' -size +1M -delete█
check: high risk: `find -delete` removes files
check: this may not be what you asked for (91% sure)
```

## Always on: checks that use no model

| Check | How |
|---|---|
| The line parses in your shell | `bash -n`, `zsh -n` or `fish -n` reads the line and runs nothing. |
| The programs exist | Every program in a simple pipeline or `&&` chain must be in `PATH` (builtins and paths are skipped). The note says it may be an alias or function, since aliases are not visible outside your shell. |
| Risk | The same rule-based classifier as the [exec tool](../exec-tool/). Medium and high risk get a note with the reason. A plain pipeline is not a finding. |

Turn them off with `[validator] enabled = false`.

## Optional: a decision model

A **decision model** is a small classifier, not a chat model. You give it a record and a typed question and it answers with a label and probabilities in one forward pass, in milliseconds, with no text generation. shellmate asks two questions about every suggestion:

- Does the command do what you asked, and nothing beyond it?
- How dangerous is it: read-only, changes things, or destructive?

A finding is shown only when the model is at least `min_confidence` sure (default 80%). Unsure answers stay silent.

It is **off by default**. To turn it on, configure a decider the way you configure a provider:

```toml
[validator]
semantic = true
deciders = ["checker"]        # tried in order

[deciders.checker]
plugin = "builtin:systemone"
trust = "remote"
[deciders.checker.settings]
preset = "openrouter"         # reads OPENROUTER_API_KEY
model = "~typesafe/jev-latest" # a "latest" alias: it follows the newest release
```

The `openrouter` preset calls OpenRouter's Decisions API, which is in alpha, so its path may change. To call TypeSafe directly instead, use `preset = "typesafe"` and `model = "jev-latest"`. To keep the check on your machine, use a decision model served locally (`preset = "ollama"`, `trust = "local"`).

`builtin:systemone` speaks the System One wire format: a `state` and typed `questions`, answered with labels and probabilities. Hosted APIs such as TypeSafe's, Ollama's decision models and [Ollaya](https://github.com/ollaya-dev/ollaya) serve it. Point it at any of them, and change the model by changing `model`.

The format is served at `/v1/systemone` below the endpoint, except where a preset knows better: `openrouter` uses `/alpha/decisions`. Set `path` to use another one. A preset's path applies only while the endpoint is the preset's own, so a custom `endpoint` gets `/v1/systemone` unless you set `path`. `preset` takes the same names as [providers](../providers/#presets-and-endpoints) (`openai`, `openrouter`, `ollama`, `lmstudio`, `localai`) with the same default key variables, and one more, `typesafe` (`https://api.typesafe.ai`, key from `TYPESAFE_API_KEY`, model `jev-latest`). Only a server that serves the format can be a decider. Use `endpoint` for any other:

```toml
[deciders.hosted]
plugin = "builtin:systemone"
trust = "remote"
[deciders.hosted.settings]
endpoint = "https://decisions.example.com"
model = "some-decision-model"
api_key = "env:DECISIONS_API_KEY"
```

### No decision model? The LLM answers

With `semantic = true` and no `deciders` (or none reachable), your normal [LLM provider](../providers/) answers the same two questions as a short JSON reply. This costs a second, small request. Turn it off with `llm_fallback = false`; then the check is skipped, visibly.

### Order of attempts

1. Each decider in `validator.deciders`, in order. A decider that is down or answers in an unreadable way is skipped.
2. The LLM, if `llm_fallback` is on.
3. Otherwise a note says the semantic check was skipped, and why.

## Privacy

- A decider gets only your request, the proposed command and the name of your shell. Nothing else from your context.
- Deciders have a `trust` like providers. A **remote** decider is refused unless a redaction layer is active, and the text sent to it goes through the redaction chain first. A cleartext `http://` endpoint is accepted only for loopback hosts. API keys are `env:` or `file:` references, never inline.
- `config check` validates the deciders without contacting them. `config show` lists them.
- The audit log records that a validation happened and how many notes it produced, never the text.

## Setting expectations

Decision models differ a lot in quality, and general-purpose ones are not trained on shell commands. In a small hand-made test with `laya`, answers to "does this command match the request?" were right about half the time, which is why the default is off, the threshold is high, and a finding is a note and never a block. Try your model on commands you know, and raise `min_confidence` if it cries wolf.

Timing: a decision model answers in about 150 ms once loaded. A model that Ollama has just loaded can take several seconds, longer than the default `timeout = "5s"`; the check is then skipped with a note, and the next one is fast. Raise `timeout` if you prefer to wait.

### Keeping the decision model loaded

Ollama unloads a model when it is idle, and by default keeps only one model loaded at a time, so using your LLM can evict the decision model between two checks. On a small GPU that makes nearly every first check slow. Two ways out:

- Give the check time to load the model: `[validator] timeout = "20s"`.
- Keep both models in memory (if they fit): start Ollama with `OLLAMA_KEEP_ALIVE=1h OLLAMA_MAX_LOADED_MODELS=2`.

`shellmate doctor` checks that each decider's endpoint accepts a connection (a TCP connect; nothing is sent, and the model is not loaded by the check) and warns when a decision model is configured with a `timeout` under 15 seconds.

## Settings

See [`[validator]` and `[deciders.<name>]`](../../reference/configuration/#validator).
