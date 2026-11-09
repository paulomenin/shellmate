---
title: Troubleshooting
description: Use shellmate doctor to find what is wrong, and fixes for the common problems.
---

## Start with `doctor`

```sh
shellmate doctor
```

```text
OK    version        shellmate 0.0.1
OK    config         ~/.config/shellmate/config.toml is valid
WARN  provider       ollama: cannot connect to localhost:11434
      -> start the server (e.g. `ollama serve`) or check the endpoint
WARN  integration    not found in ~/.zshrc
      -> run `shellmate install-shell`
```

`doctor` checks the version, the config, whether each provider endpoint accepts a connection (a TCP connect; nothing is sent), whether each configured decision model accepts a connection, the shell integration, the conversation store, the audit path, the locale, the `man` tool and that your shortcuts parse. Every line that is not OK says how to fix it. It exits with status 1 only if something FAILs, and `--json` gives machine-readable output.

## Common problems

### "shellmate isn't configured yet"

You pressed a shortcut before a provider was set up. Run `shellmate setup`, or `shellmate config init` for a starter file.

### Pressing the key does nothing

- Is the integration loaded? `doctor` reports whether the rc file contains it. Open a new shell after installing.
- Is the key taken? When the shell starts, shellmate prints a notice if a binding already exists and leaves it alone. Choose another key in `[input.shortcuts]`.
- In some terminals `alt` is not sent as Meta. Look for an "Option as Meta" setting (macOS) or try a `ctrl+` shortcut.
- With bash, make sure the line sits at the **end** of `~/.bashrc`, after starship or other prompt tools, and that bash is 4.4 or newer.

### Git context or syntax checks time out on a slow machine

The helper programs have short fixed limits (git status 0.6 s, the shell syntax check 2 s, `man` 5 s). On a slow disk or a huge repository, set `SHELLMATE_TIME_SCALE=3` (1 to 100) in the environment to stretch them all. It never shortens a limit.

### The first answer is very slow

Local models must be loaded into memory on first use; raise `timeout_secs`. Reasoning models also "think" before they write: set `reasoning_effort = "none"` in the provider settings.

### The provider cannot be reached

Check that the server is running and that the endpoint is right. For Ollama, `curl http://localhost:11434/v1/models` should answer. Cleartext `http://` is only accepted for loopback hosts. Use `https://` for remote servers.

### A remote provider is refused for lack of redaction

You see an error with the code `privacy.no_redaction`: a `trust = "remote"` provider is refused when the redaction layer is not active. Restore the default pipeline (`[pipeline] llm = ["..."]`) or mark a provider that really is on your machine as `trust = "local"`.

### The config is rejected

The loader is strict. Unknown keys, inline secrets and a `file:` secret that is group or world readable are all errors, and the message names the key. Check with `shellmate config check`; it validates provider settings and secret references without any network access.

### A `cmd:` secret fails

The error names the key and the program, for example ``providers.work.settings.api_key`: `pass` failed (exit 1)``. Run `shellmate doctor` to run the program once and see the result, and `shellmate config check` to check the line without running it.

- `config.invalid_cmd`: the line has shell syntax (a pipe, `;`, `$VAR`, a glob, a backslash) or a relative program path. Put the pipeline in a script and call the script.
- `config.secret_cmd_missing`: the program is not in `PATH` or is not executable. `PATH` here is the one your shell exports.
- `config.secret_cmd_failed`: the program exited with an error, printed more than one line, printed too much, or took longer than `[secrets] cmd_timeout`. The program gets a small environment, so a variable it needs (`PASSWORD_STORE_DIR`, `OP_SERVICE_ACCOUNT_TOKEN`) must be listed in `[secrets] pass_env`.
- `config.secret_unset`: the program printed nothing.

shellmate does not try the next provider after one of these, because the key is something to fix.

### Follow-ups do not remember anything

- Conversations are per shell. A new terminal starts fresh.
- After 30 idle minutes a new conversation starts.
- Turns that used level-2 context are not replayed to remote providers. `--explain` shows how many turns were omitted.
- `[conversation] store = "none"` turns them off.

### No manual pages

Install `man-db` or `mandoc`, or disable the tool with `[tools.man] enabled = false`.

## Still stuck?

`shellmate config show` prints the effective configuration, and `shellmate audit show` lists what happened. Both are content-free and safe to paste into an issue at <https://github.com/paulomenin/shellmate/issues>.
