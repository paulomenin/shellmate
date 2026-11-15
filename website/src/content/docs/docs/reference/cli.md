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
shellmate setup --yes --provider openrouter --model "~openai/gpt-luna-latest" --shell fish
```

Without `--yes`, setup is a conversation. It first looks at the machine: which hosted services have their key variable set (by name only, never the value) and which local servers answer on their default port. It lists the providers and says for each whether it is local or remote and what it found, asks you to choose, and then:

1. For a local server, it confirms the port or takes another one.
2. For a remote service, it shows what would be sent and asks you to agree. It does this before anything is contacted.
3. It asks where the API key is, as `env:NAME` or `file:PATH`. A key pasted by mistake is refused. When the service's variable is already set, it does not ask.
4. It asks the server for its list of models, to confirm the address and the key. Only the key is sent. A failure offers to try again, to continue anyway, or to cancel.
5. It asks for the model name. The name is never guessed. One that is not in the server's list is kept with a note, because aliases are often not listed.

It then does the same for an optional **checker** (a decision model that judges each suggested command), listing only the ones it found: a key variable that is set for OpenRouter or TypeSafe, or a local Ollama.

With `--yes`, nothing is asked and nothing is contacted. `--provider` and `--model` are required.

| Option | Meaning |
|---|---|
| `--yes` | Don't ask; use the given options. |
| `--provider <P>` | `openrouter`, `openai`, `ollama`, `lmstudio`, `localai` or `custom`. |
| `--model <M>` | The model name, as the provider lists it. |
| `--base-url <URL>` | The base URL of the server. Required for `custom`. With a preset it replaces the preset's address, and the preset's key variable no longer applies. |
| `--api-key <REF>` | Where the key is: `env:NAME` or `file:PATH`. A value that is not a reference is refused. |
| `--api-key-env <NAME>` | The same as `--api-key env:NAME`. |
| `--checker <P>` | Also set up a checker: `openrouter`, `ollama`, `typesafe` or `custom`. |
| `--checker-model <M>` | The checker's model. Required with `--checker`. |
| `--checker-url <URL>` | The checker's base URL. Required for `custom`. |
| `--checker-api-key <REF>` | Where the checker's key is: `env:NAME` or `file:PATH`. |
| `--verify` | With `--yes`: ask each server for its models first, to confirm the address and the key. This sends the key, and a failure stops setup before anything is written. |
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
| `config edit` | Open the config file in `$VISUAL`, then `$EDITOR`, then `vi`, and run `config check` on the file when the editor closes. The editor line may carry arguments (`code --wait`) and is started directly, never through a shell. Your edit is kept even if the file no longer loads. It does not create a file: use `config init` first. |
| `config init [--provider P --model M] [--force]` | Write a documented starting config. Without options the provider examples are comments and you choose one. `--provider` and `--model` go together and write an active config. Never overwrites an existing file without `--force`. |

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
