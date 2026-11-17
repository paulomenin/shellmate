---
title: The LLM pipeline
description: How a request is built for each provider, how the middleware chain wraps the call, how redaction works, and how fallback decides when to try the next provider.
---

The "pipeline" is everything between a finished prompt and a stream of text coming back. It has three parts: the **request builder** (one payload per provider), the **`llm` middleware chain** (redaction and anything else you add), and the **provider chain** with its fallback rules.

```mermaid
flowchart TB
    subgraph per["for each model in [llm] chain"]
        build["build_request<br/>level, history, tool offers, system prompt"]
        subgraph chain["llm middleware chain (outermost first)"]
            m1["layer 1<br/>builtin:redact"] --> m2["layer 2 ..."]
        end
        term["provider.chat_stream"]
        build --> m1
        m2 --> term
        term -. "stream" .-> m2
        m2 -. "stream, restored" .-> out["engine reads the stream"]
    end
    out --> tools{"tool call<br/>in the reply?"}
    tools -- "yes" --> next["append result, go round again"]
    next --> build
    tools -- "no" --> done["final answer"]
```

## 1. One payload per provider

The request is **rebuilt for every provider**, and nothing prepared for one provider is reused for another. This is what keeps the privacy rules honest. `build_request` decides:

- **Context level.** The smaller of `[context] default_level` and the provider's ceiling (`max_context_level`, or 3 for `local` and 1 for `remote`).
- **History.** Earlier turns, filtered by the provider's trust. A `remote` provider is never shown turns that carried level-2 context. It is told how many were left out.
- **Tool instructions.** Only the tools that are enabled and allowed for this action. See [Tool calling](../tool-calling/).
- **The system prompt and the user message.** See [Context and prompts](../context-and-prompts/).
- **Taint.** Whether the request carries text other people wrote, which suspends your pre-approval rules for the turn.

The request also carries the conversation's **redaction key**, so the same secret gets the same placeholder in every turn of one conversation.

`shellmate --dry-run` and `shellmate context` run exactly this step, then stop before the provider.

## 2. The middleware chain

Middleware is an onion. Layers are ordered **outermost first**, as written in `[pipeline] llm`. A layer receives the request and a handle to the rest of the chain. It may change the request, call `next` (once, or not at all if it answers by itself) and change what comes back. `--dry-run` and `shellmate context` report an error when a layer does not pass a request on, since there is then nothing to show.

```text
request  -> layer 1 -> layer 2 -> provider
stream   <- layer 1 <- layer 2 <- provider
```

The innermost handler is the **terminal**. In a real request it calls the provider's `chat_stream`. In `--dry-run` it is a recorder that keeps the request instead of sending it, which is how the preview can show the true wire payload after redaction.

The chain runs **on every provider call**: each round of the tool loop (so tool results are redacted too), each fallback provider, and the other model-shaped calls below.

| Call | Goes through the chain? |
|---|---|
| A provider call for the answer | Yes |
| The next round after a tool result | Yes |
| A fallback model | Yes |
| A chat model answering the semantic check questions | Yes |
| A decision model on a `remote` provider | Yes, its record passes through the chain first |
| A decision model on a `local` provider | No |
| A tool running a program | No (nothing leaves the machine) |

### `builtin:redact`

The built-in redactor has the `Redactor` role. On the way down it replaces each secret it finds in every message with a stable placeholder like `<SECRET:a91f3c>`. On the way up it puts the original value back into the streamed answer, so a command the model suggests stays valid and the secret itself never leaves the machine.

What it looks for:

| Rule | Examples |
|---|---|
| Private key blocks | `-----BEGIN ... PRIVATE KEY-----` |
| Passwords in URLs | `https://user:password@host` |
| Well-known token formats | `sk-...`, `ghp_...`, `github_pat_...`, `xoxb-...`, `AKIA...`, `AIza...`, `glpat-...`, `npm_...`, `hf_...` |
| JSON web tokens | `eyJ....` |
| Auth headers | `Authorization: Bearer ...`, `Basic ...`, `token ...` |
| Command-line credentials | `curl -u user:pass`, `--password x`, `--token x`, `--api-key x` |
| Assignments | `API_KEY=...`, `set -x TOKEN ...` for secret-looking names |
| Your own rules | `[plugins."builtin:redact"] extra_patterns` |

Values that are never secrets are left alone: shell variables (`$TOKEN`), `env:` and `file:` references, existing placeholders, and `true`, `false`, `yes`, `no`, `null`, `0`, `1`.

Restoring is done on a stream, so a placeholder can arrive split across chunks. The restorer holds back any trailing text that could still be the start of a placeholder (`<`, `<SEC`, `<SECRET:ab`) until the next chunk decides it.

:::caution
Redaction is best-effort pattern matching. It reduces accidental leaks. It is not a data-loss-prevention guarantee, and it cannot hide a secret that does not match a rule. See [Privacy and redaction](../../guides/privacy/).
:::

## 3. The privacy gate (INV-2)

`check_privacy` runs when the engine is built and again at the start of every run and every preview. It refuses to continue if:

- any **provider** has `trust = "remote"`, or any **decider** has `trust = "remote"`, and
- no active middleware layer has the `Redactor` role.

The error is `privacy.no_redaction`, and the hint says to enable `builtin:redact` or to set `trust = "local"` for a provider that runs on your machine. "Active" means listed in the pipeline and not disabled in `[plugins]`. `trust` defaults to `remote`, so an unmarked provider is treated as remote.

## 4. The provider chain and fallback

`[llm] chain` lists chat models, primary first. The engine tries them in order and **only falls back before anything has been shown or run**.

```mermaid
flowchart TD
    start["try provider N"] --> ok{"answered?"}
    ok -- "yes" --> done["use this answer"]
    ok -- "no" --> shown{"did any text reach you,<br/>or did a tool run?"}
    shown -- "yes" --> fail["fail: the error is final"]
    shown -- "no" --> kind{"error is timeout,<br/>unavailable or rate limited?"}
    kind -- "no" --> fail
    kind -- "yes" --> more{"more providers?"}
    more -- "yes" --> start
    more -- "no" --> all["fail: all N providers failed; last: ..."]
```

Why: text that has been shown cannot be taken back, and a tool that has run cannot be undone, so a failure after that point is final. The errors that allow fallback are `provider.timeout`, `provider.unavailable` and `provider.rate_limited`. Bad credentials (`provider.auth`), a bad request, a content filter or any other error stop the request, because trying another provider would not fix them.

Each fallback is a full rebuild (section 1), so a fallback to a less trusted provider gets a smaller context, less history and its own redaction.

### Lazy secrets

A `cmd:` secret runs a program such as `pass` or `op`, which can take a while. So the engine builds a provider twice: once at start-up with a placeholder in place of the secret (this checks every other setting and is what `info` answers from), and once the first time the provider is used, with the program's real output. A fallback provider that is never called never pays for it. A failure to fetch the secret is a configuration error with its own code, reported before anything is sent.

### The built-in provider

`builtin:openai-compatible` posts to `/v1/chat/completions` below the endpoint (a base URL with no version) and reads the reply as a server-sent-events stream. Settings: `preset` (`openai`, `openrouter`, `ollama`, `lmstudio`, `localai`) or `endpoint`, `model`, `api_key` (a reference), `timeout_secs` (default 60) and `reasoning_effort`.

HTTP errors map to error kinds, and the kind decides fallback:

| Status | Kind | Falls back? |
|---|---|---|
| Client timeout, `408` | Timeout | Yes |
| Connection failed, `5xx` | Unavailable | Yes |
| `429` | Rate limited | Yes |
| `401`, `403` | Auth | No |
| `400`, `404`, `413`, `422` | Bad request | No |
| Anything else | Other | No |

Error messages never include a URL, so credentials embedded in one stay out of what you see.

## 5. Reading the stream

The engine reads the stream event by event:

- `Text` events are added to the full reply and passed through the display filter (see [Output and presentation](../output-and-presentation/)).
- `Usage` events are summed across rounds for `--explain`.
- `Done` or the end of the stream finishes the round.
- An error ends the round. If text had already been produced, the error is final.

Dropping the future (Ctrl-C) drops the stream, which closes the connection.
