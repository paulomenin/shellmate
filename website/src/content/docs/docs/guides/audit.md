---
title: Audit log and --explain
description: See what shellmate did on your behalf without recording what you asked or what it answered.
---

## The audit log

shellmate keeps a log of **what it did**: which provider it called, how long it took, which tools were used and whether they were allowed. It never records your prompts, the answers, or tool results. A test checks that a question and its answer cannot appear in the log.

```sh
shellmate audit show     # recent events
shellmate audit export   # all events as JSON, one per line
shellmate audit clear    # delete the log
```

### Events

Every event has a version, a timestamp, the event name, the shell session, the conversation and a request id that ties the events of one request together.

| Event | Records |
|---|---|
| `request_started` | The action, the provider chain and how many earlier turns the conversation holds. |
| `llm_call` | Provider, status (`ok` or an error code), latency, time to first token, token counts, and whether the next provider was tried as a fallback. |
| `tool_call` | The tool name, the size of the request in bytes and the decision: `allow` (a `man` page), `preapproved` (one of your rules), `approved` (you said yes), `denied_by_user`, `denied_no_terminal` (nobody could be asked), `to_buffer` (you took the command to your own prompt) or `deny` (refused without asking). Never the request text or the result. |
| `validation` | How many notes the command check produced and who answered the semantic check (`off`, `model:<name>`, `failed` or `timeout`). Never the notes themselves. |
| `turn_finished` | The outcome, the audience (local-only or not), the context level, the duration and whether a command was placed in the buffer. |

### Where it lives

By default in `$XDG_RUNTIME_DIR/shellmate/audit.jsonl` (tmpfs, gone at logout). To keep it across logins:

```toml
[audit]
enabled = true
persist = true        # $XDG_STATE_HOME/shellmate/audit/audit.jsonl
retention = "30d"     # older events are pruned when new ones are written
max_bytes = 10485760  # 10 MiB cap
```

Set `enabled = false` to turn it off. A failure to write the log is silent and never blocks a request.

## `--explain`

Add `--explain` to `invoke` and, after the answer, shellmate prints what it used:

- the provider chain and which provider answered, with its trust
- the context level and whether redaction was on
- the conversation id, the turn number and how many earlier turns were omitted
- the tools used
- token counts, total model time and time to the first token

This is the quickest way to answer "why was that slow?" or "did that go to the cloud?".

## `--timings`

`--timings` prints one line after the answer with where the time went:

```
timings:  startup 3.2 ms · context 5.6 ms · llm 1 420.1 ms (first text 380.4 ms) · checks 1.7 ms · overhead 0.2 ms · total 1 431.8 ms
```

*Start-up* is reading the config and building the providers. *Context* is the project and git collectors. *llm* is everything between sending the request and the end of the answer, tool runs and approvals included, for the provider that answered. *Checks* is the validation of the suggested command, and it is not counted in *llm*. *Overhead* is whatever the engine spent outside those, which includes the time lost to a provider that failed before the fallback answered. `--timings=json` prints the same as one JSON object, which is what the [benchmarks](../../benchmarks/) read.

## `--dry-run`

`--dry-run` shows the request that would be sent, after redaction, without contacting anyone. See [Privacy and redaction](../privacy/).
