---
title: Letting the model read a program's help
description: The help tool, which runs a program's --help for the model, and how to allow a program in advance.
---

Many programs have no manual page, and a model that guesses their options is often wrong. The `help` tool lets the model read what the program says about itself. It runs `<program> --help`, or `<program> <subcommand> --help`, and sends the text back as reference data.

It is on by default, and **nothing runs unless you say yes**, unless you wrote a rule for that program.

## What the model can ask for

The model writes `help git commit` or just `help docker`. That is all it can choose: the program and up to three plain subcommand words. It cannot pass an option, a path, a quote or anything with shell syntax. The option is always `--help`, and `-h` is tried only when `--help` printed nothing.

The program must be a plain name found in your `PATH`. `./tool` and `/usr/bin/tool` are refused.

## What you see

The prompt shows the exact command that will run, `git commit --help`, and asks the same way as for the [exec tool](../exec-tool/): one `y` key runs it, and Enter, Esc, Ctrl-C, pasted text and a timeout deny. High-risk programs (the classifier is the same one `exec` uses) need the word `yes`. There is no "put it in my prompt" choice, because the help text is for the model.

## Allowing a program in advance

A rule names a program, and it covers that program's subcommands:

```toml
[tools.help]
preapproved = ["git", "docker", "cargo"]
```

- A rule is a program name. `git` covers `git --help` and `git commit --help`. A `*` matches within the name (`docker*`). A rule made only of wildcards, or one with a space in it, matches nothing, and `shellmate config check` says so.
- **The classifier keeps its veto.** If you list a program it flags, you are still asked.
- The model is told which programs need no question, so it picks them first.
- A pre-approved run is never silent: you see `ran (pre-approved: git): git commit --help`, and it is recorded in the audit log without the program or its text.

## Untrusted text

The help text was written by the program, so it counts as untrusted: the request is tainted, and your rules stop applying for the rest of that turn. A later call asks again. See [the security model](../../concepts/security-model/).

## Limits

The program runs directly, never through a shell, with an environment reduced to `PATH`, `HOME` and the locale, a 5 second timeout (the whole process group is stopped) and at most 1 MiB of output read. At most `max_bytes` of the text goes to the model.

## Settings

```toml
[tools.help]
enabled = true          # default
approval = "ask"        # "deny" turns the tool off. There is no "allow".
preapproved = []        # program names
timeout = "5s"
max_bytes = 8192        # most help text sent to the model
```

Turning the tool off removes its instructions from the prompt, so the model is not told it exists. It is offered for `ask`, `generate` and `explain`.

## When to use `man` instead

If a manual page exists, `man` is better: it is complete, and it needs no question. The model is told to use `help` for programs without a page.
