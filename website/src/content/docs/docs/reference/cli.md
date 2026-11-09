---
title: Command line
description: Every shellmate command and option.
---

```text
shellmate [--config <PATH>] <COMMAND>
```

`--config <PATH>` (or the `SHELLMATE_CONFIG` environment variable) uses another config file instead of `~/.config/shellmate/config.toml`. It is accepted by every command.

## `setup`

Guided first-time setup: writes the config, installs the shell integration and runs `doctor`.

```sh
shellmate setup
shellmate setup --yes --provider ollama --model qwen2.5-coder:7b --shell fish
```

| Option | Meaning |
|---|---|
| `--yes` | Don't ask; use the given options. |
| `--provider <P>` | `ollama`, `lmstudio`, `localai`, `openai`, `openrouter` or `custom`. |
| `--base-url <URL>` | With `custom`: the base URL of the OpenAI-compatible server. |
| `--api-key-env <NAME>` | With `custom`: the environment variable that holds its API key. |
| `--model <M>` | The model name. |
| `--shell <S>` | `fish`, `zsh` or `bash`. |

## `install-shell [SHELL]`

Adds, updates or removes the marked block in your rc file. `SHELL` defaults to your login shell.

| Option | Meaning |
|---|---|
| `--dry-run` | Show the change without applying it. |
| `--yes` | Apply without a terminal to confirm. |
| `--remove` | Delete the block. |

## `doctor`

Checks the installation and says how to fix problems. Exit status 1 if any check FAILs.

| Option | Meaning |
|---|---|
| `--json` | Machine-readable output. |

## `config`

| Command | Meaning |
|---|---|
| `config check` | Validate the config, including provider settings and secret references. No network access. |
| `config show` | Print the effective configuration. Secrets appear only as references. |
| `config init [--provider P] [--model M] [--force]` | Write a documented starting config (default provider `ollama`). Never overwrites an existing file without `--force`. |

## `conversation`

Follow-ups share the shell session's conversation. See [Follow-up conversations](../../guides/conversations/).

| Command | Meaning |
|---|---|
| `conversation new` | Start a new conversation for this shell session. |
| `conversation show` | Print the current conversation, including tool calls. |
| `conversation list` | List stored conversations; the current one is marked. |
| `conversation resume <ID>` | Make a stored conversation the current one. |
| `conversation undo` | Forget the last turn. |
| `conversation clear [--all]` | Delete the current conversation, or all of them. |

`--session <SESSION>` (or `SHELLMATE_SESSION`) selects the shell session; the integration sets it for you.

## `audit`

| Command | Meaning |
|---|---|
| `audit show` | Print recent events. |
| `audit export` | Print events as JSON. |
| `audit clear` | Delete the log. |

See [Audit log and --explain](../../guides/audit/).

## `context [TEXT...]`

Shows which pieces of context each provider in your chain would receive for a request typed here, and why the others are withheld. Sizes and reasons only, and nothing is sent. See [Privacy and redaction](../../guides/privacy/#see-what-is-sent).

| Option | Meaning |
|---|---|
| `--action <ask\|generate\|explain>` | The action to preview (default `ask`). |
| `--full` | Also print the exact payload after redaction. |

## `render [FILE]`

Shows markdown the way an answer would be shown, to try your terminal and your [`[ui]` settings](../../guides/output/). Reads standard input when there is no file.

| Option | Meaning |
|---|---|
| `--color <auto\|plain\|mono\|16>` | How much styling to use. `auto` decides like an answer does (default). |
| `--width <COLUMNS>` | Where to wrap (default: your terminal's width). |
| `--ascii` | ASCII symbols instead of Unicode ones. |
| `--no-markdown` | Show the text as written, keeping the colours. |

## `check [--shell SH] [--json] <LINE>`

Checks a command line the way shellmate checks every suggestion, without running it: does it parse in the shell, do its programs exist, how risky does the classifier think it is. Exit status 0 means no findings, 1 means there were; with `--json` it prints one object and always exits 0.

## `init <SHELL>`

Prints the shell integration for `fish`, `zsh` or `bash`, with your shortcuts bound. This is what the rc-file block runs.

## `invoke`

Runs one action. The shell glue calls it with the request on standard input. You normally do not run it by hand, but it is useful for debugging.

| Option | Meaning |
|---|---|
| `--dry-run` | Show the request that would be sent, after redaction, and exit without contacting any provider. |
| `--explain` | After the answer, print what was used: provider, context, tools, tokens, timing. |
| `--timings[=text\|json]` | After the answer, print where the time went: start-up, context, the model (and when the first text arrived), command checks, the client's own overhead, total. `json` is one line, for scripts. |

The input is a NUL-separated list of `key=value` fields with `buffer` last:

```sh
printf 'v=1\0action=ask\0shell=zsh\0cwd=%s\0buffer=%s\0' "$PWD" 'how do I list hidden files?' \
  | shellmate invoke --explain
```

| Exit status | Meaning |
|---|---|
| 0 | Replace the input line with standard output. |
| 10 | Leave the input line as it is. |
| other | An error. |

## Environment variables

| Variable | Meaning |
|---|---|
| `SHELLMATE_CONFIG` | Path of the config file. |
| `SHELLMATE_SESSION` | Shell session id; exported by the integration. |
| `XDG_CONFIG_HOME` | Where the config lives (default `~/.config`). |
| `XDG_RUNTIME_DIR` | Conversations and the default audit log live under it. |
| `XDG_STATE_HOME` | The persistent audit log lives under it. |
| `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE` | Colour: see [Colours and formatting](../../guides/output/#when-it-falls-back-to-plain-text). `[ui] color` overrides them. |
