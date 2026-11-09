---
title: Providers
description: Configure local and remote LLM providers, API keys, trust levels, and fallbacks.
---

A **provider** is a named instance of a provider plugin plus its settings. The only plugin today is `builtin:openai-compatible`, which talks to any server that implements the OpenAI chat-completions API (with streaming).

## Local: Ollama and LM Studio

```toml
[providers.ollama]
plugin = "builtin:openai-compatible"
trust = "local"
[providers.ollama.settings]
preset = "ollama"
model = "qwen2.5-coder:7b"
timeout_secs = 180        # the first request may have to load the model
reasoning_effort = "none" # see the note below
```

## Remote: OpenRouter, OpenAI and others

```toml
[providers.cloud]
plugin = "builtin:openai-compatible"
trust = "remote"          # this is the default when omitted
[providers.cloud.settings]
preset = "openrouter"
model = "your-provider/model-name"   # as listed by the provider
# No api_key needed: the openrouter preset reads OPENROUTER_API_KEY from your environment.
# To use another variable or a file, set it explicitly:
# api_key = "env:MY_KEY_VARIABLE"      (or "file:/path/to/key", mode 0600)
```

## Presets and endpoints

A preset supplies a default endpoint and, for hosted services, the environment variable that holds the API key. If you set `endpoint`, it wins, and the preset's key variable is no longer used (see [below](#default-variables-for-presets)).

| Preset | Endpoint | Key variable (when `api_key` is not set) |
|---|---|---|
| `openai` | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` |
| `ollama` | `http://localhost:11434/v1` | none |
| `lmstudio` | `http://localhost:1234/v1` | none |
| `localai` | `http://localhost:8080/v1` | none |

For anything else, set the endpoint yourself:

```toml
[providers.vllm.settings]
endpoint = "http://localhost:8000/v1"
model = "my-model"
```

Cleartext `http://` is accepted **only for loopback hosts**. Any other server must use `https://`.

## Settings

| Key | Meaning |
|---|---|
| `preset` / `endpoint` | One of them is required. |
| `model` | Required. The model name the server knows. |
| `api_key` | A secret reference (below). Optional: the `openai` and `openrouter` presets read `OPENAI_API_KEY` and `OPENROUTER_API_KEY` when it is absent. |
| `timeout_secs` | Request timeout. |
| `reasoning_effort` | Sent as `reasoning_effort` (`none`, `low`, ...). Reasoning models spend most of their time thinking silently; for short shell answers `none` is typically 2 to 3 times faster. |

Unknown keys are errors, so typos fail loudly.

## API keys are references, never values

`api_key` must be a reference:

- `env:NAME` reads an environment variable of the shell that runs the shortcut.
- `file:/path` reads a file and trims it. The file must be mode 0600 or tighter.
- `cmd:program args` runs a program and uses what it prints. See [Ask a program for the key](#ask-a-program-for-the-key).
- `keyring:service/account` reads the system keyring. See [Use the system keyring](#use-the-system-keyring).

An environment variable is the simplest form, but it means the key sits in a shell startup file, and every program you start inherits it. If you keep keys in a password manager or your system keyring, `cmd:` reads them from there instead, so no key is written to a file or exported.

### Ask a program for the key

```toml
[providers.work.settings]
preset  = "openai"
model   = "gpt-5"
api_key = "cmd:pass show api/openai"
```

Other examples:

```toml
api_key = "cmd:op read op://Private/OpenAI/credential"             # 1Password CLI
api_key = "cmd:keepassxc-cli show -q -s -a Password --no-password -k /home/me/.keys/vault.key /home/me/vault.kdbx openai"   # KeePassXC, key-file database
api_key = "cmd:age -d -i ~/.age/key.txt /home/me/.secrets/openai.age"
api_key = "cmd:/home/me/bin/openai-key"                            # your own script
```

For the system keyring, see the next section, which is shorter.

How it works:

- **It runs when the provider is first used**, not when shellmate starts. A fallback provider that is never called never runs its program, and `config check` and `--dry-run` do not run it. A key is fetched once per request, because every request is a new process, so a slow program makes every request slower. `doctor` shows how long it took.
- **The program prints the key and nothing else**: one line of text, at most 4096 bytes. Empty output, several lines, a non-zero exit, or too much output is an error. `pass show` prints every line of an entry, so keep the key alone in the entry, or point `cmd:` at a script that prints the first line.
- **No shell is involved.** The line is split into words, with `'single'` and `"double"` quotes, and `~/` at the start of a word means your home directory. Pipes, `;`, `&&`, redirects, `$VARIABLES`, `` `backticks` ``, globs and backslashes are refused, so a line never does something other than what it says. For a pipeline, put it in a script, or write it out yourself with `cmd:sh -c 'pass show x | head -n1'`: you are then asking for a shell on purpose.
- **The program** is an absolute path, or a bare name found in `PATH`. A relative path such as `./get-key` is refused. Empty and relative `PATH` entries are skipped.
- **It gets a small environment**: `PATH`, `HOME`, `USER`, `LANG`, the locale, `TMPDIR`, and what a keyring or GnuPG agent needs to be found (`XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `GNUPGHOME`, `GPG_TTY`). Your other variables, including `OPENAI_API_KEY`, are not passed. List more in `[secrets] pass_env`, for example `["PASSWORD_STORE_DIR"]`.
- **It has no input**, and it is stopped after `[secrets] cmd_timeout` (5 seconds by default). It stays in the terminal's foreground process group, so a passphrase prompt on the terminal should be answerable within the timeout. This has not been tried with a real `pinentry` yet; a graphical pinentry or a running agent is the safer choice. Raise `cmd_timeout` if you need time to type.
- **A failure is a configuration error.** It names the key (`providers.work.settings.api_key`) and the program, and shows the program's error output, shortened. shellmate does **not** fall back to the next provider, because a broken key is something to fix, not to route around.
- **The key is never shown.** Messages name the program and never print its arguments or output, and `config show` prints the line you wrote. If an argument looks like a key written in place (`cmd:echo sk-...`), `config check` warns and `config show` hides the arguments. Put the key in a password manager, and let `cmd:` name where it is.

The `cmd:` line comes from your own config file only. It runs with your permissions, like a shell alias, so protect the file as you protect `~/.ssh`. `cmd:` works in provider and decision-model settings. For a decision model, and for plugin settings, the program runs when shellmate starts and not on first use.

### Use the system keyring

```toml
api_key = "keyring:shellmate/openai"
```

`keyring:service/account` is short for the keyring tool of your system, run as a `cmd:` line:

| System | Tool | Same as |
|---|---|---|
| Linux (GNOME Keyring, KWallet) | `secret-tool` from libsecret | `cmd:secret-tool lookup service shellmate account openai` |
| macOS (Keychain) | `security` | `cmd:security find-generic-password -s shellmate -a openai -w` |

Store the key once, outside shellmate:

```sh
secret-tool store --label="shellmate openai" service shellmate account openai   # Linux
security add-generic-password -s shellmate -a openai -w                         # macOS
```

Everything above for `cmd:` applies: it runs on first use, with a small environment and a timeout, and the tool must be in `PATH`. The service and the account may use letters, digits and `. _ @ : + -`, and not start with `-`. Other systems are not supported; use `cmd:`, `env:` or `file:` there.
KeePassXC has two routes. If you turn on Secret Service integration in its settings and expose a group, `secret-tool` can read your entries, so `keyring:` may work on Linux (check with `secret-tool lookup service shellmate account openai` first). Otherwise use `keepassxc-cli` as in the example above. A `cmd:` program gets no input, so `keepassxc-cli` cannot ask for the master password: the example needs a database that opens with a key file alone, and that file must be mode 0600.

### Default variables for presets

With `preset = "openai"` or `"openrouter"` and no `api_key`, shellmate reads `OPENAI_API_KEY` or `OPENROUTER_API_KEY` from the environment. An explicit `api_key` always wins. If you set a custom `endpoint`, the preset's variable is **not** used, so a key is never sent to a server you did not set it up for. Write `api_key = "env:NAME"` yourself in that case. If the variable is unset, no key is sent and the provider answers with its authentication error. `ollama`, `lmstudio` and `localai` need no key.

An inline key in the config is rejected by the loader. `shellmate config show` prints the reference you wrote, never the resolved value, and `shellmate doctor` checks that references resolve without printing them.


## Trust and context levels

`trust` decides how much context the provider may receive and which conversation turns may be replayed to it.

| `trust` | Default `max_context_level` |
|---|---|
| `local` | 3 |
| `remote` | 1 |

Override it per provider with `max_context_level`. The level actually used is the smaller of `[context] default_level` and the provider's maximum. A `remote` provider is **refused** unless the redaction layer is active, which it is by default.

## Fallback chains

```toml
[llm]
chain = ["cloud", "ollama"]    # the first is primary, the rest are fallbacks in order
```

If a provider fails before anything has been shown (offline, timeout, error status), the next one is tried. For each provider shellmate rebuilds the whole request: the context level is capped to that provider's maximum, the conversation history is assembled for that provider's trust, and redaction is applied. Nothing prepared for one provider ever reaches another. Once text has been shown to you, there is no fallback.

## Tip: gemma and other reasoning models

If the first answer is slow, the model is probably "thinking" before it writes. Set `reasoning_effort = "none"`. In testing with a local `gemma4:e4b`, this cut latency by 2 to 3 times; the first request after loading the model can still take a while, which is what `timeout_secs` is for.
