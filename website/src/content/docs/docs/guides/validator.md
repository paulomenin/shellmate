---
title: Checking suggested commands
description: Notes shown after the answer, just above the suggested command, and optional models that judge whether it matches your request.
---

Before a suggested command reaches your input line, shellmate checks it. The checks only add notes after the answer, above the input line that now holds the command. **The command is always still placed in your input line**, and you decide.

```text
$ list python files bigger than 1MB            ← alt+i
check: high risk: `find -delete` removes files
check: this may not be what you asked for (91% sure)
$ find . -name '*.py' -size +1M -delete█
```

## Always on: checks that use no model

| Check | How |
|---|---|
| The line parses in your shell | `bash -n`, `zsh -n` or `fish -n` reads the line and runs nothing. |
| The programs exist | Every program in a simple pipeline or `&&` chain must be in `PATH` (builtins and paths are skipped). The note says it may be an alias or function, since aliases are not visible outside your shell. |
| Risk | The same rule-based classifier as the [exec tool](../exec-tool/). Medium and high risk get a note with the reason. A plain pipeline is not a finding. |

Turn them off with `[validator] enabled = false`.

## Optional: a model that judges the command

A **decision model** is a small classifier, not a chat model. You give it a record and a typed question and it answers with a label and probabilities in one forward pass, in milliseconds, with no text generation. shellmate asks two questions about every suggestion:

- Does the command do what you asked, and nothing beyond it?
- How dangerous is it: read-only, changes things, or destructive?

A finding is shown only when the model is at least `min_confidence` sure (default 80%). Unsure answers stay silent.

It is **off by default**. To turn it on, define a decision model and put it in `[validator] chain`. A decision model is a `[models.<name>]` with `plugin = "builtin:systemone"`, reached through a [provider](../providers/), like a chat model:

```toml
[providers.openrouter]
trust = "remote"
preset = "openrouter"         # reads OPENROUTER_API_KEY

[models.checker]
provider = "openrouter"
plugin = "builtin:systemone"
model = "~typesafe/jev-latest" # a "latest" alias: it follows the newest release

[validator]
chain = ["checker"]           # tried in order
```

The provider can be the same one your chat model uses. A model on a provider shares its trust, endpoint and key.

The `openrouter` preset calls OpenRouter's Decisions API, which is in alpha, so its path may change. To call TypeSafe directly instead, use a provider with `preset = "typesafe"` and a model `jev-latest`. To keep the check on your machine, use a decision model served locally (a provider with `preset = "ollama"` and `trust = "local"`).

`builtin:systemone` speaks the System One wire format: a `state` and typed `questions`, answered with labels and probabilities. Hosted APIs such as TypeSafe's, Ollama's decision models and [Ollaya](https://github.com/ollaya-dev/ollaya) serve it. Point it at any of them, and change the model by changing `model`.

The format is served at `/v1/systemone` below the endpoint, except where a preset knows better: `openrouter` uses `/alpha/decisions`. Set `path` on the model to use another one. A preset's path applies only while the endpoint is the preset's own, so a custom `endpoint` gets `/v1/systemone` unless you set `path`. `preset` takes the same names as [providers](../providers/#presets-and-endpoints) (`openai`, `openrouter`, `ollama`, `lmstudio`, `localai`) with the same default key variables, and one more, `typesafe` (`https://api.typesafe.ai`, key from `TYPESAFE_API_KEY`, model `jev-latest`). Only a server that serves the format can host a decision model. Use `endpoint` on the provider for any other:

```toml
[providers.hosted]
trust = "remote"
endpoint = "https://decisions.example.com"
api_key = "env:DECISIONS_API_KEY"

[models.hosted]
provider = "hosted"
plugin = "builtin:systemone"
model = "some-decision-model"
```

### A chat model can judge too

A chat model in `[validator] chain` answers the same two questions as a short JSON reply. This costs a small extra request. It is how you get a check without a decision model, and it works as a fallback after one:

```toml
[validator]
chain = ["checker", "qwen"]   # the decision model first, then a chat model
```

Nothing asks a chat model unless you list it. With an empty chain there is no semantic check.

### Order of attempts

1. Each model in `[validator] chain`, in order. One that is down or answers in an unreadable way is skipped.
2. If none answers, a note says the semantic check was skipped, and why.

## Privacy

- A model in the chain gets only your request, the proposed command and the name of your shell. Nothing else from your context.
- A model takes its `trust` from its provider. A model on a **remote** provider is refused unless a redaction layer is active, and the text sent to it goes through the redaction chain first. A cleartext `http://` endpoint is accepted only for loopback hosts. API keys are `env:` or `file:` references, never inline.
- `config check` validates the models without contacting them. `config show` lists them.
- The audit log records that a validation happened and how many notes it produced, never the text.

## Setting expectations

Decision models differ a lot in quality, and general-purpose ones are not trained on shell commands. In a small hand-made test with `laya`, answers to "does this command match the request?" were right about half the time, which is why the default is off, the threshold is high, and a finding is a note and never a block. Try your model on commands you know, and raise `min_confidence` if it cries wolf.

Timing: a decision model answers in about 150 ms once loaded. A model that Ollama has just loaded can take several seconds, longer than the default `timeout = "5s"`; the check is then skipped with a note, and the next one is fast. Raise `timeout` if you prefer to wait.

### Keeping the decision model loaded

Ollama unloads a model when it is idle, and by default keeps only one model loaded at a time, so using your LLM can evict the decision model between two checks. On a small GPU that makes nearly every first check slow. Two ways out:

- Give the check time to load the model: `[validator] timeout = "20s"`.
- Keep both models in memory (if they fit): start Ollama with `OLLAMA_KEEP_ALIVE=1h OLLAMA_MAX_LOADED_MODELS=2`.

`shellmate doctor` checks that each decision model's endpoint accepts a connection (a TCP connect; nothing is sent, and the model is not loaded by the check) and warns when a decision model is configured with a `timeout` under 15 seconds.

## Settings

See [`[validator]`](../../reference/configuration/#validator) and [`[models.<name>]`](../../reference/configuration/#modelsname).
