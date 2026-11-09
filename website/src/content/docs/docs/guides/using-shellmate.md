---
title: Using shellmate
description: The ask, generate and explain actions, shortcuts, and what happens to your input line.
---

You type in your normal prompt and press a shortcut. shellmate sends your text, plus a little context, to the model and shows the result. The one rule: **a command it suggests goes into your input line and is never executed.** You read it, edit it if you like, and press Enter yourself.

## The actions

| Action | Default key | Input | Result |
|---|---|---|---|
| `ask` | `alt+m` | A question in plain language | The answer streams below the prompt. If it contains a command, that command is placed in your input line. |
| `generate` | `alt+i` (suggested) | A description of what you want | Command only. Exactly one command is placed in your input line and nothing is printed, no explanation and no alternatives. If one command cannot do it, or the request is ambiguous, you get one short sentence instead (a reason or a question) and your line is left alone. |
| `explain` | `alt+j` (suggested) | A command | A breakdown streams below the prompt. Your line is left untouched. |
| `new-conversation` | `alt+k` (suggested) | none | Starts a fresh [conversation](../conversations/) for this shell. |
| `capture` | `alt+r` (suggested; in bash that key is readline's `revert-line`, so use `alt+e`) | A command | Wraps the line as `shellmate run -- ...` so the next question can use its output. No model call, nothing runs. See [Asking about a command's output](../capture/). |

Only `alt+m` is bound by default. The others are suggestions that `setup` writes into your config; bind whatever you like (see below).

`ask` and `generate` are different. `ask` shows the model's explanation and places the first command it suggests in your input line, listing any others under "Other options". The prompt tells the model that one block is one thing to run: steps that go together come in that block, joined with `&&` when a step needs the one before it (or on separate lines when they are independent), and a second block is only for a real alternative. A command that spans several lines is placed in the input line as it is and runs as written when you press Enter. `generate` shows only the command, and only the first one. Output is not streamed because there is nothing to read while it works. Use `generate` when you know what you want and will judge the command yourself, `ask` when you want the reasoning, and `explain` on a command you did not write.

```text
$ list the five biggest files in this directory
                                              ← alt+i
$ du -ah . | sort -rh | head -5█
↳ lists entries by size, largest first, keeps the top five
```

## Choosing your shortcuts

```toml
[input.shortcuts]
"alt+m" = "ask"
"alt+i" = "generate"
"alt+j" = "explain"
"alt+k" = "new-conversation"
"alt+r" = "capture"
```

- Keys are written the same way for every shell: `alt+m`, `ctrl+x`, `ctrl+alt+j`.
- A shortcut needs `ctrl` or `alt` (a bare letter would stop you typing it), and a single letter or digit as the key. Chords such as `ctrl+x ctrl+e` are not supported yet.
- shellmate never overrides an existing binding. If the key is taken it prints a notice when the shell starts and leaves your binding alone. Pick another key.

## While it works

- Output is **appended** below your prompt and scrollback-safe. There is no full-screen UI and the terminal is left as it was found.
- After 300 ms of silence a small spinner appears; it is removed before the first word of the answer.
- `Ctrl-C` cancels. The connection to the model is dropped so generation stops, `[cancelled]` is printed, and your buffer stays intact.
- **An Enter you press while waiting never runs the suggestion.** Whatever you type while shellmate is working is thrown away, whatever the outcome (answer, `explain`, error or cancel), so a stray Enter cannot execute a line you have not reviewed. You see a one-line note when an Enter was discarded. Keys you press after the result appears are yours as usual.
- Text from the model is sanitized before it is shown: control characters and terminal escape sequences are stripped, so an answer cannot change your window title or touch your clipboard.

## What the model knows about you

By default shellmate sends your text and a small amount of context, depending on the **context level**:

| Level | Added to your text |
|---|---|
| 0 | Nothing. |
| 1 (default) | Shell and version, OS, working directory. |
| 2 | Also the exit status of the last command and your recent commands (5 by default). |

Secrets are replaced by placeholders before anything is sent. A remote provider is capped at level 1 unless you raise it. See [Privacy and redaction](../privacy/).

## Per-action settings

```toml
[actions.explain]
conversation = "fresh"     # every explain starts from scratch
idle_timeout = "5m"
```

`conversation` is `auto` (the default), `continue` or `fresh`; see [Follow-up conversations](../conversations/).

## Disabling streaming

```toml
[ui]
stream = false    # print the answer once it is complete
```
