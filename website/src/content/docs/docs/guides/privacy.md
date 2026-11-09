---
title: Privacy and redaction
description: What shellmate sends to a provider, how secrets are replaced by placeholders, and how to inspect a request before it leaves your machine.
---

shellmate is built so that you can decide, per provider, how much of your shell it may see. Three mechanisms work together: **context levels**, **reversible redaction**, and **inspection**.

## Context levels

| Level | What is sent besides your text |
|---|---|
| 0 | Nothing. |
| 1 | Shell and version, OS, working directory, and the kind of project (file names only). |
| 2 | The last exit status, recent commands (`history_lines`, 5 by default), the git branch and change counts, and [captured command output](../capture/) if you used `shellmate run`. |

```toml
[context]
default_level = 1
history_lines = 5
```

Each provider has a ceiling (`max_context_level`) that comes from its `trust`: 3 for `local`, 1 for `remote`. The level used is the smaller of the two. Raising a remote provider above 1 is an explicit decision you make in the config.

## Reversible redaction

The built-in redaction layer sits closest to the provider. Before a request leaves, it replaces secrets with stable placeholders such as `<SECRET:a1b2c3>`. When the answer streams back, the placeholders are turned into your real values again, so a suggested command still works.

It looks at what is about to be sent: your text, the working directory, and the recent commands. It recognizes common key and token formats, `Authorization` headers, `KEY=value` assignments for names like `*TOKEN*` or `*PASSWORD*`, and similar patterns. Add your own rules with a regular expression; if the pattern has a group named `secret`, only that part is replaced.

```toml
[plugins."builtin:redact".settings]
extra_patterns = [
  { name = "internal-hosts", regex = '[a-z0-9-]+\.corp\.example\.com' },
]
```

Properties worth knowing:

- **Stable within a conversation.** Each conversation has its own random key, so the same secret keeps the same placeholder in every turn.
- **Fails closed.** If the redaction layer cannot do its job, the request is not sent.
- **Best effort.** Redaction cannot know about a secret format it has never seen, or a secret you paste into free text in an unusual shape. Treat it as a safety net, not a guarantee. For sensitive work, use a local provider.

## Remote providers need redaction

A provider with `trust = "remote"` is **refused** unless a redaction layer is active in the pipeline. This is enforced in code, not by convention. If you replace the pipeline, keep it:

```toml
[pipeline]
llm = ["..."]    # "..." stands for the built-in defaults, which include builtin:redact
```

## See what is sent

`shellmate context` lists, for each provider in your chain, every piece of context and whether it would be sent:

```
provider `ollama` (local) · context level 2 (ceiling 3) · redaction on
  ✓ L0 your typed text    22 chars
  ✓ L1 shell and system   zsh on Arch Linux
  ✓ L1 working directory  ~/projects/shell-ai
  ✓ L1 project type       Rust (cargo), mise (file names only)
  · L2 last exit status   the shell did not report one
  ✓ L2 git state          branch and change counts
  · L2 captured output    nothing captured (wrap a command with `shellmate run`)
```

A withheld item says why: the level it needs, the `[context]` setting that turns it off, or that there was nothing to send. The view shows sizes and reasons, never content, and nothing is sent. It builds the context from where you run it, with the text you pass (`shellmate context why is it slow`); `--action generate` previews another action, and `--full` also prints the exact payload after redaction. The last exit status and recent commands are known only when a shortcut calls shellmate, so they show as not reported in this view.

## Inspect before you trust

`--dry-run` shows the exact request that would be sent, after redaction, without contacting any provider:

```sh
printf 'v=1\0action=ask\0shell=zsh\0cwd=/home/me/project\0buffer=why did my build fail?\0' \
  | shellmate invoke --dry-run
```

`--explain` runs the request and then prints what was used: which provider, trust, context level, whether redaction was on, which conversation and turn, which tools, token counts and timing. See [Audit log and --explain](../audit/).

## Conversations and remote providers

Turns that included level-2 context (your commands) are local-only. They are never replayed to a remote provider. You are told when turns are omitted. See [Follow-up conversations](../conversations/).

## What is stored and where

| Data | Where | Lifetime |
|---|---|---|
| Conversations | tmpfs (`$XDG_RUNTIME_DIR/shellmate`), mode 0600 | Until logout, or cleared |
| Audit log | tmpfs by default; `$XDG_STATE_HOME` if you set `persist = true` | Until logout, or the retention you configure |
| Config | `~/.config/shellmate/config.toml` | Yours. Holds secret *references*, never secret values |

shellmate has no telemetry of any kind.
