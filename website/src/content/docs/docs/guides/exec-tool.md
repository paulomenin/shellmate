---
title: Letting the model run commands
description: The exec tool, the approval prompt, the classifier and your own pre-approval rules.
---

With `ask`, the model can **look at your machine** before answering: run `git status`, `systemctl status sshd`, `ls`, and read the output. This is the `exec` tool. It is on by default, and **nothing runs unless you say yes**.

Commands you only want to *run yourself* are different: `generate` and `ask` put them in your input line and never run them. `exec` exists so the model can gather information, not to do your work.

## What you see

When the model asks to run something, shellmate shows the exact command and stops:

```text
╭─ shellmate wants to run a command ─────────────────────
│  $ systemctl status sshd
│  in: /home/you/project
│  risk: low
│
│  [y] run   [n] deny   [b] put it in my prompt (Enter, Esc and Ctrl-C deny)
╰─ choice (n):
```

| Key | Meaning |
|---|---|
| `y` | Run it, this once |
| `n`, Enter, Esc, Ctrl-C | Deny. The model is told and carries on without it |
| `b` | Do not run it, but put the command in your input line for you to run |

Only a single `y` key press approves. Pasted text, arrow keys and everything else do not. Keys you typed before the prompt appeared are thrown away, so they cannot answer it. A prompt that nobody answers for two minutes denies. With no terminal at all (a pipe, a script) every request is denied.

The command is shown whole and escaped, so control characters or look-alike characters cannot disguise it.

After it runs you see one line with the exit status and the first lines of output. The output also goes back to the model as reference data, never as instructions, and passes through [redaction](../privacy/) like everything else that is sent.

## What can run

`exec` runs **one simple command**: plain words and quotes. No pipes, redirects, variables, globs, backticks, `&&`, `cd` or other shell syntax. The model is told this and, if it tries anyway, the call is refused without bothering you. Simple commands are run directly with the program found in your `PATH`, not through a shell, so what you approve is exactly what executes.

A command runs in your current directory with no input, a 30 second timeout (the whole process group is stopped), at most 16 KiB of output, and a small fixed environment: your `PATH`, `HOME` and locale, but none of your other variables.

## Risky commands

A built-in, rule-based classifier looks at every command. It never approves anything. It can only make things stricter:

| Risk | Examples |
|---|---|
| high (you must type `yes`) | `rm`, `mv`, `dd`, `chmod`, `sudo`, `kill`, restarting services, installing packages, programs that run other programs (`sh`, `python`, `xargs`, `env`, `find -exec`), commands that touch `~/.ssh`, `.env` and other credential paths |
| medium | `curl`, `ssh` and other network programs, `git push` and other commands that change a repository, `tar` creating or extracting |
| low | everything it does not recognize, such as `ls`, `git status`, `df` |

At high risk the prompt shows why and asks you to type `yes` and press Enter instead of one key.

## Pre-approving commands

For lookups you are happy to allow, write rules in your config. This is the only way to approve something in advance. There is no "always allow" key in the prompt.

```toml
[tools.exec]
preapproved = ["ls *", "pwd", "git status", "systemctl status *"]
```

- A rule matches the whole command word by word. `git status` matches only `git status`, not `git status -s`. A trailing `*` matches any remaining arguments, including none.
- The program must be a plain name found in `PATH`. `./ls` never matches.
- **The classifier keeps its veto.** If you write `rm *` and the model asks for `rm file`, you are still asked. A rule only skips the prompt for commands the classifier has no objection to.
- To let the model read a program's help without asking, name the program: `git --help` and `git * --help` cover `git --help` and `git commit --help`. A rule such as `* --help` is not accepted, because the program must be named. The help text was written by the program, so it counts as untrusted: once a command's output is in the request, the remaining calls of that turn ask again.
- A pre-approved run is never silent: you see `✔ ran (pre-approved: git status)` and it is recorded in the audit log.

`shellmate config check` warns about rules that are too broad, such as `*` or `sh *`.

## Settings

```toml
[tools.exec]
enabled = true          # default
approval = "ask"        # "deny" turns the tool off. There is no "allow".
preapproved = []        # your rules
timeout = "30s"
max_output = 16384      # bytes kept for stdout and for stderr
env = "sanitized"       # or "inherit" to pass your whole environment
```

`exec` is offered to `ask`, `generate` and `explain`. Every command still asks you first. In `explain` the prompt has no "put it in my prompt" choice, because that action never places a command in your input line.

## What is not covered

- **Remote providers see the output.** What a command prints is sent to the model, redacted on a best-effort basis. Do not approve commands that print private data to a remote provider. The classifier flags credential paths, but it cannot know what an arbitrary command prints.
- **You are the last line of defense.** The classifier is a list of rules, not a sandbox. Read what you approve.
- Pipelines and redirects cannot run, even with your approval. The model can suggest them in your input line instead.
