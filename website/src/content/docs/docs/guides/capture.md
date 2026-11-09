---
title: Asking about a command's output
description: Wrap a command with shellmate run so your next question can use what it printed.
---

"Why did that fail?" needs the output of the command that failed. shellmate does not record your terminal. You choose which commands to keep, by running them through `shellmate run`.

```sh
$ shellmate run -- cargo build
   Compiling foo v0.1.0
error[E0425]: cannot find value `x` in this scope

$ why did that fail?            ← alt+m
```

The next question carries the output of the wrapped command, so you don't paste anything.

## The capture shortcut

You rarely know a command will matter before you run it, so wrap it as you type. Bind the `capture` action:

```toml
[input.shortcuts]
"alt+r" = "capture"
```

Type a command and press the key. The line becomes `shellmate run -- <your command>`; nothing is run and no model is called. Press Enter to run it as usual. Press the key again to take the wrapper off.

- A simple command is wrapped directly: `shellmate run -- cargo build`.
- A line that needs a shell (pipes, redirects, variables) becomes `shellmate run --shell zsh -- 'ls | wc -l'`, using the shell you are in. Aliases and functions are not available there.
- Multi-line input is not wrapped.

## What is kept

- **One command at a time.** Each `shellmate run` replaces the previous capture for the same shell session.
- **Private and temporary.** It is a file in your runtime directory (tmpfs), readable only by you, per shell session. It disappears at logout or reboot.
- **Bounded.** At most 64 KiB is stored (the start and the end of long output). At most `[context] capture_bytes` (default 8192) is sent.
- **Fresh only.** A capture older than an hour is ignored.
- **Cleaned.** Colors and terminal control codes are removed, and progress bars that rewrite one line become separate lines.

## When it is sent

Captured output is sent only at **context level 2 or higher**, and never above the level a provider is allowed (`max_context_level`). A remote provider at level 1 never sees it. It goes through [redaction](../privacy/) with the rest of the request, so secrets in the output are replaced by placeholders. `--dry-run` shows exactly what would be sent.

## Safety

The output is text a program chose to print. A file, a web page or a log line in it could contain instructions aimed at the model. shellmate labels it as reference data, and treats a request that carries it as **tainted**: your `preapproved` rules for the [exec tool](../exec-tool/) are suspended for that request, so every command the model asks to run needs your approval at the prompt.

## Limits

- The command's output goes through pipes, so programs that check for a terminal may turn colors off or skip interactive behavior. Wrap commands that print and exit; do not wrap `vim`, `ssh` or `less`.
- stdout and stderr are kept together in the order they arrived.
- Ctrl-C stops the command and the output printed so far is still saved.
- The command's exit status is the exit status of `shellmate run`, and shown to the model.
