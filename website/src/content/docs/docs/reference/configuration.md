---
title: Configuration
description: Every section and key of config.toml.
---

shellmate reads `~/.config/shellmate/config.toml` (or `$XDG_CONFIG_HOME/shellmate/config.toml`). Use `--config <path>` or the `SHELLMATE_CONFIG` environment variable for another file. Only what differs from the built-in defaults needs to be written.

```sh
shellmate config init     # write a documented starter file
shellmate config check    # validate everything, with no network access
shellmate config show     # print the effective settings (secrets only as references)
```

## Rules

- **TOML.** Comments are welcome.
- **Strict.** Unknown keys are errors, so typos fail loudly.
- **Secrets are references**, never values: `env:NAME`, `file:/path` (the file must be mode 0600 or tighter), `keyring:service/account` (the system keyring) or `cmd:program args` (a program prints the key; see [Providers](/docs/guides/providers/#ask-a-program-for-the-key)). The loader rejects inline secrets, and `config show` never prints resolved values.
- **No project-local config.** A file found in the current directory is never loaded. A config can enable tools, and loading one from a cloned repository would be a trust problem.
- **Durations** need a unit: `30s`, `5m`, `2h`, `7d`.

## `version`

```toml
version = 1
```

Required. Unknown versions are rejected with a message.

## `[providers.<name>]`

| Key | Default | Meaning |
|---|---|---|
| `plugin` | none | The provider plugin. Today: `builtin:openai-compatible`. |
| `trust` | `remote` | `local` or `remote`. Sets the default context ceiling and decides whether conversation turns may be replayed. |
| `max_context_level` | 3 (local), 1 (remote) | Ceiling for the context level sent to this provider. |
| `settings.*` | | Plugin settings, validated strictly. |

Settings of `builtin:openai-compatible`: `preset`, `endpoint`, `model`, `api_key`, `timeout_secs`, `reasoning_effort`. The `openai` and `openrouter` presets read `OPENAI_API_KEY` and `OPENROUTER_API_KEY` when `api_key` is not set. See [Providers](../../guides/providers/).

## `[llm]`

| Key | Default | Meaning |
|---|---|---|
| `chain` | `[]` | Provider names, primary first, then fallbacks in order. |

## `[validator]`

See [Checking suggested commands](../../guides/validator/).

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Deterministic checks: the line parses in your shell, programs exist, risk notes. |
| `semantic` | `false` | Also ask a decision model (or the LLM) whether the command matches your request and how risky it is. |
| `deciders` | `[]` | Names from `[deciders.<name>]`, tried in order. Empty means the LLM answers. |
| `llm_fallback` | `true` | When no decider answers, let the LLM provider chain answer the same questions. |
| `timeout` | `5s` | Longest wait for the semantic check. On timeout it is skipped with a note. |
| `min_confidence` | `0.8` | A semantic finding is shown only when the model is at least this sure (0.5 to 1). |

## `[deciders.<name>]`

Decision-model backends, configured like providers.

| Key | Default | Meaning |
|---|---|---|
| `plugin` | none | Today: `builtin:systemone` (`POST {endpoint}/v1/systemone`). |
| `trust` | `remote` | `local` or `remote`. A remote decider needs an active redaction layer. |
| `settings.*` | | `preset` (`ollama`) or `endpoint`, `model`, `api_key` (`env:`, `file:`, `keyring:` or `cmd:` reference), `timeout_secs` (default 10). |

## `[context]`

| Key | Default | Meaning |
|---|---|---|
| `default_level` | `1` | 0 nothing extra, 1 shell/OS/cwd, 2 plus last status and recent commands. Capped by each provider's `max_context_level`. |
| `history_lines` | `5` | Recent commands sent at level 2 and above. |
| `project` | `true` | Level 1 and up: the kind of project (file names only). |
| `git` | `true` | Level 2 and up: branch, ahead/behind, change counts. |
| `git_log_lines` | `0` | Also send this many recent commit subjects (0 to 20). They are text others wrote, so they taint the request. |
| `capture_bytes` | `8192` | Most bytes of output captured with `shellmate run` that are sent, at level 2 and above. See [Asking about a command's output](../../guides/capture/). |

## `[input.shortcuts]`

A table of `"key" = "action"`. Keys use `ctrl+` and `alt+` with a single letter or digit. Actions: `ask`, `generate`, `explain`, `new-conversation`, `capture`. Default: `"alt+m" = "ask"`.

## `[pipeline]`

| Key | Default | Meaning |
|---|---|---|
| `llm` | `["..."]` | Middleware around the provider call, outermost first. `"..."` stands for the built-in defaults, which include `builtin:redact`. A list without `"..."` replaces the defaults entirely. |

## `[conversation]`

| Key | Default | Meaning |
|---|---|---|
| `store` | `builtin:store-tmpfs` | The store plugin, or `none` to turn follow-ups off. |
| `idle_timeout` | `30m` | A conversation idle for longer is replaced. |
| `max_turns` | `40` | Turns kept in the store. |
| `max_tool_rounds` | `8` | Tool calls allowed per request. |
| `history.strategy` | `window` | How history is reduced (`summarize` is not available yet). |
| `history.keep_recent_turns` | `6` | Turns replayed to the model. |
| `history.elide_tool_results_after` | `2` | Older tool output is shortened. |

## `[actions.<name>]`

For `ask`, `generate` and `explain`:

| Key | Default | Meaning |
|---|---|---|
| `conversation` | `auto` | `auto`, `continue` or `fresh`. |
| `idle_timeout` | global value | Overrides `[conversation] idle_timeout` for this action. |

## `[tools.man]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Whether the model may read manual pages. |
| `approval` | `allow` | `allow` or `deny`. |
| `max_bytes` | `8192` | Longest page text sent to the model. |

## `[tools.exec]`

See [Letting the model run commands](../../guides/exec-tool/).

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Whether the model may ask to run commands (with `ask`). |
| `approval` | `ask` | `ask` or `deny`. `allow` does not exist: use `preapproved`. |
| `preapproved` | `[]` | Rules for commands that run without a prompt, for example `"git status"` or `"ls *"`. The classifier can still force a prompt. |
| `timeout` | `30s` | A command running longer is stopped. |
| `max_output` | `16384` | Bytes kept from stdout and from stderr. |
| `env` | `sanitized` | `sanitized` or `inherit`. |

## `[tools.read]`

See [Letting the model read files](../../guides/read-tool/).

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Whether the model may ask to read a file (with `ask`). |
| `approval` | `ask` | `ask` or `deny`. |
| `preapproved` | `[]` | Path rules (`*` and `?`, relative to the working directory) read without a prompt. Credentials are never covered. |
| `roots` | `[]` | Directories besides the working directory that may be read (absolute, or starting with `~/`). |
| `max_bytes` | `16384` | Most file text sent to the model. |

## `[secrets]`

How `cmd:` references run.

| Key | Default | Meaning |
|---|---|---|
| `cmd_timeout` | `"5s"` | How long a `cmd:` program may take, between `1s` and `120s`. |
| `pass_env` | `[]` | Names of environment variables to hand to a `cmd:` program, besides the fixed list in the [Providers guide](/docs/guides/providers/#ask-a-program-for-the-key). |

```toml
[secrets]
cmd_timeout = "15s"                       # a gpg passphrase prompt needs time
pass_env    = ["PASSWORD_STORE_DIR"]
```

## `[audit]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Write the audit log. |
| `persist` | `false` | Keep it in `$XDG_STATE_HOME` instead of tmpfs. |
| `retention` | `30d` | Events older than this are pruned. |
| `max_bytes` | 10 MiB | Size cap. |

## `[ui]`

| Key | Default | Meaning |
|---|---|---|
| `stream` | `true` | Show the answer while it is generated. Tool activity (commands run, files read, refusals) is always shown, whatever this is set to. |
| `color` | `auto` | `auto`, `always` or `never`. `auto` uses colour on a terminal and falls back to plain text otherwise, and follows `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE` and `TERM`. See [Colours and formatting](../../guides/output/). |
| `markdown` | `true` | Format the markdown in answers (bold, lists, code) where colour or styling is on. |

## `[runtime]`

| Key | Default | Meaning |
|---|---|---|
| `mode` | `oneshot` | The binary starts per request. A background daemon is planned. |

## `[plugins."<id>"]`

Per-plugin switch and settings, for example the redaction layer:

```toml
[plugins."builtin:redact"]
enabled = true
[plugins."builtin:redact".settings]
extra_patterns = [{ name = "internal-hosts", regex = '[a-z0-9-]+\.corp\.example\.com' }]

[plugins."builtin:store-tmpfs".settings]
dir = "/run/user/1000/shellmate-conversations"
```

## A fuller example

```toml
version = 1

[providers.cloud]
plugin = "builtin:openai-compatible"
trust = "remote"
[providers.cloud.settings]
preset = "openrouter"
model = "your/model-name"
api_key = "env:OPENROUTER_API_KEY"

[providers.ollama]
plugin = "builtin:openai-compatible"
trust = "local"
[providers.ollama.settings]
preset = "ollama"
model = "qwen2.5-coder:7b"
timeout_secs = 180

[llm]
chain = ["cloud", "ollama"]

[context]
default_level = 1

[input.shortcuts]
"alt+m" = "ask"
"alt+i" = "generate"
"alt+j" = "explain"
"alt+k" = "new-conversation"

[conversation]
idle_timeout = "30m"

[actions.explain]
conversation = "fresh"

[audit]
persist = true
retention = "14d"
```
