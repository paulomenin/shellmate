---
title: Context and prompts
description: What the model is told about your shell, how context levels and provider trust limit it, where each piece comes from, and how the prompt is put together for each action.
---

Every request sends the model a **system message** (rules, then what shellmate knows about your shell), the **history** of the conversation, and the **user message** (your input line). This page explains where each piece comes from and which providers may see it.

## The actions

The glue names an action in the envelope. Four reach the engine and one is handled by the binary.

| Action | Default key | What it does | Calls a model? | Tools offered | Leaves a command in the input line? |
|---|---|---|---|---|---|
| `ask` | `alt+m` | Answers a question. May suggest a command. | Yes | `man`, `exec`, `read` | The first command block, if any |
| `generate` | `alt+i` | Turns a request into exactly one command. The model's own words are not shown, unless it asks a clarifying question. | Yes | `man` | The command |
| `explain` | `alt+j` | Explains the command in the input line: what it does, each part, the risk. | Yes | `man` | No |
| `new-conversation` | `alt+k` (suggested) | Forgets the session's conversation. | No | none | No |
| `capture` | `alt+r` (suggested) | Wraps the line as `shellmate run -- ...`, or unwraps it. | No | none | The rewritten line |

The built-in default knows only `alt+m` for `ask`. The starter config that `shellmate setup` and `config init` write adds `alt+i` and `alt+j`, and has `alt+k` and `alt+r` commented out for you to enable. Shortcuts live in `[input.shortcuts]`.

An empty input line is refused (`input.empty`) before anything is sent.

## Context levels

A **level** is how much of your shell the model may see. `[context] default_level` sets the level you want (0 to 4, default 1). Each provider has a ceiling, from its `max_context_level` or, by default, 3 for `local` and 1 for `remote`. The level used for a request is the **smaller of the two**, worked out again for each provider in the chain.

| Level | Added to the system message | Comes from |
|---|---|---|
| 0 | Nothing. Only your text. | |
| 1 | Shell and its version, OS, working directory, the kind of project, and syntax notes for your shell | The envelope, `/etc/os-release`, the project collector |
| 2 | The last exit status, recent commands, git state, and output captured with `shellmate run` | The envelope, the git collector, the capture file |
| 3 and 4 | Accepted by the config, but today they add nothing beyond level 2 | |

The working directory is sent with your home directory replaced by `~`, so your user name is not sent needlessly.

Two further rules follow from the level:

- A turn that was answered at level 2 or above is **local-only**: it is never replayed to a `remote` provider later. See [State on disk](../state-and-storage/).
- A request that carries captured output or commit subjects at level 2 or above is **tainted**. See [Tool calling](../tool-calling/).

## Where each piece comes from

### The envelope

The glue sends the shell name and version, the working directory, the cursor, the last exit status and command, how long it took, a ring of the last 10 commands with their status, the session id, and the input line. `[context] history_lines` (default 5) says how many of those commands are used. See [Shell glue](../glue-protocol/).

### The project collector

At level 1 and above, if `[context] project` is on, shellmate looks in the working directory, and in its parent directories up to the repository root or your home directory, for **file names** from a fixed list: `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `Makefile`, `Dockerfile`, `flake.nix`, lockfiles and a few dozen more. It reports what kind of project it is ("Rust (cargo)", "Node.js", "pnpm"). It never reads file contents, and the words it can produce come from a fixed vocabulary, so nothing written by a third party can reach the prompt through it.

### The git collector

At level 2 and above, if `[context] git` is on, shellmate runs `git status` and reports the branch, ahead and behind counts, and counts of changes. This is hardened, because a cloned repository's own config can name programs for git to run (`core.fsmonitor`): that setting is overridden on the command line. The command has a short limit (600 ms by default) and runs on a blocking thread, so the async runtime keeps going.

`[context] git_log_lines` (default 0) adds that many recent commit subjects. Subjects are text other people wrote, so they are escaped, labeled as data, and taint the request.

### Captured output

`shellmate run -- <command>` runs a command normally and keeps a bounded copy of its output in a private tmpfs file for the session. At level 2 and above the next request may include it, cut to `[context] capture_bytes` (default 8192) and only if it is less than an hour old. It is redacted like everything else, and it taints the request. See [Asking about a command's output](../../guides/capture/).

## How the prompt is assembled

`prompt::build_with` produces the messages for one provider:

```text
system   rules + context (by level) + action instructions + tool offers
...      history: earlier turns this provider may see
user     your input line
```

The **system message** is built from, in order:

1. Who shellmate is, and to be concise because the answer is read in a terminal.
2. At level 1 and above: the shell, version, OS, working directory, shell syntax notes (fish, zsh, bash, or plain POSIX), and the project kind.
3. At level 2 and above: the last exit status, the recent commands, the git section and the captured output.
4. The convention for suggesting a command: exactly one command in its own block labeled `shellmate-command`, joined with `&&` or on separate lines for steps that belong together, and a second block only for a real alternative.
5. The tool offers, one per tool that is available for this action (see [Tool calling](../tool-calling/)).
6. A note, if earlier turns were left out because this provider is remote.
7. The action's own instructions: answer the question, produce exactly one command, or explain the command.

The **user message** is your input line. For `explain` it is "Explain this command:" followed by the line. If the line is exactly the command shellmate suggested last, the message adds a note saying so, so the model understands a follow-up such as "make it recursive".

The context is **rebuilt every turn and never stored**. Only the user message, the tool calls and results, and the answer are saved in the conversation.

## What you can look at

| Command | Shows |
|---|---|
| `shellmate context <text>` | Which pieces of context each provider would receive and why the others are held back. Sizes and reasons, never content. |
| `shellmate context --full <text>` | Also the exact payload after redaction. |
| `shellmate invoke --dry-run` | The real wire payload for every provider in the chain. Nothing is sent. |
| `shellmate invoke --explain` | After a request: provider, trust, level, redaction, conversation, tools and token counts. |

The last exit status and recent commands are known only when a shortcut calls shellmate, so `shellmate context` run by hand shows them as not reported.
