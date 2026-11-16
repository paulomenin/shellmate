# shellmate

A companion for your shell: type a question or a request, press a shortcut, and get an answer or a command **in your prompt** (never run for you). Works in fish, zsh and bash. Answers stream as they are generated. Rust, MIT.

> Status: early development (milestones M1-M5 largely done: fish/zsh/bash glue, streaming, redaction, follow-up conversations, `man` tool, setup/doctor, audit log). Design documents live in `docs/` (currently git-ignored).

Documentation site (Astro Starlight, static): see [`website/`](website/) — `cd website && pnpm install && pnpm dev`.

## Quick start

```sh
cargo build --release
target/release/shellmate setup        # picks a provider, writes the config, adds one block to your shell's rc file
target/release/shellmate doctor       # checks everything and says how to fix what is wrong
```

Non-interactive: `shellmate setup --yes --provider openrouter --model "~openai/gpt-luna-latest" --shell fish`. Other commands: `config show | init | edit | check`, `install-shell [--remove]`, `audit show | export | clear`, `invoke --explain`.

## Try it by hand

```sh
cargo build --release
mkdir -p ~/.config/shellmate
cat > ~/.config/shellmate/config.toml <<'EOF2'
version = 1
[providers.openrouter]
trust = "remote"             # "local" for a server on this machine (Ollama, LM Studio, LocalAI)
preset = "openrouter"        # reads OPENROUTER_API_KEY; or: endpoint = "https://..." + api_key = "env:MY_API_KEY"
[models.main]
provider = "openrouter"
plugin = "builtin:openai-compatible"
model = "~openai/gpt-luna-latest"
# reasoning_effort = "none"  # much faster with reasoning models
[llm]
chain = ["main"]
[input.shortcuts]
"alt+m" = "ask"
"alt+i" = "generate"
"alt+j" = "explain"
EOF2
target/release/shellmate config check
```

Then add one line (with the absolute path of the binary) to your shell's rc file:

| Shell | Line |
|---|---|
| fish | `/path/to/shellmate init fish \| source` in `~/.config/fish/config.fish` |
| zsh | `source <(/path/to/shellmate init zsh)` in `~/.zshrc` |
| bash (>= 4.4) | `eval "$(/path/to/shellmate init bash)"` at the **end** of `~/.bashrc` (after prompt tools such as starship) |

Type `how do I find the biggest files here` and press `alt+m`. The answer streams below the prompt; a suggested command is placed in your input line for you to review and run. `alt+i` turns the line into one command, `alt+j` explains the command you typed. `Ctrl-C` cancels a request. `shellmate invoke --dry-run` shows exactly what would be sent (after redaction).

## Follow-ups

Press the shortcut again and the model remembers the earlier turns of this shell (`how do I find big files` → `same, but only .log files`). Each interactive shell has its own conversation, kept in `$XDG_RUNTIME_DIR` (tmpfs, gone at logout), and a conversation idle for 30 minutes is replaced by a new one. Turns that used your recent commands (context level 2+) are never replayed to a `trust = "remote"` provider; you are told when turns are left out. Secrets keep the same placeholder in every turn.

```sh
shellmate conversation show      # what the model remembers, including tool calls
shellmate conversation undo      # forget the last turn
shellmate conversation new       # start over (or bind a key: "alt+k" = "new-conversation")
shellmate conversation list | resume <id> | clear [--all]
```

```toml
[conversation]
idle_timeout = "30m"     # also per action: [actions.explain] idle_timeout = "5m"
store = "builtin:store-tmpfs"   # or "none" to turn follow-ups off
[actions.explain]
conversation = "fresh"   # auto (default) | continue | fresh
[tools.man]
enabled = true           # lets the model read a manual page before answering (read-only, plain page names only)
```

## Development

Tool versions (Node, pnpm, Rust) are pinned in `mise.toml` and `mise.lock`: run `mise install`. System packages for the shell tests (fish, zsh, tmux, python3, starship) are not managed by mise.

```sh
cargo test --workspace          # unit + binary tests (mock provider, no network)
tools/e2e/run-all.sh            # real fish, zsh and bash in tmux (with and without starship) against a mock provider
tools/e2e/conformance.sh zsh    # one shell
```

See the [contributing guide](website/src/content/docs/docs/contributing.md) for the checks, the test rules and the safety invariants. Coding agents should start with [`AGENTS.md`](AGENTS.md).
