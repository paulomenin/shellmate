---
title: Letting the model read files
description: The read tool, what it may and may never read, and the project and git context.
---

With `ask`, the model can read **one text file** from your working directory before it answers: a config, a log, a source file. This is the `read` tool. It is on by default, and **no file is read unless you say yes**.

## What you see

```text
╭─ shellmate wants to read a file ─────────────
│  read: nginx.conf
│  in: ~/work/site
│  ! 1842 bytes; the text is sent to `cloud` (remote, after redaction)
│
│  [y] read it   [n] deny (Enter, Esc and Ctrl-C deny)
╰─ choice (n):
```

The prompt names the file and says where its text will go. Only a single `y` key reads it. Enter, Esc, Ctrl-C, pasted text and a timeout deny, and keys typed before the prompt appeared are thrown away, exactly as for the [exec tool](../exec-tool/).

## What it may read

Checked before anyone is asked:

- **Inside the working directory** (or a directory you list in `roots`). Symlinks are resolved first, so a link cannot lead out. A path that is missing and a path that is outside get the same answer, so the model cannot probe your disk.
- **A regular, text file.** Directories and binary files are refused. Control characters and terminal escape sequences are removed.
- **Never credentials.** Anything that looks like keys or credentials is refused with no prompt and cannot be unlocked by a rule: `.env`, `.ssh`, `.aws`, `.gnupg`, `.netrc`, `.git/config`, `id_rsa`, `*.pem`, names containing `secrets` or `credentials`, `/proc`, `/dev`, and similar. The name of your own project directory is not judged, only what is below it.
- **Size-capped.** At most `max_bytes` (16 KiB) is sent; a longer file is cut and says so.

Redaction still applies to what is sent, so a token inside an ordinary file is replaced by a placeholder.

## Your own rules

```toml
[tools.read]
preapproved = ["docs/*", "*.md"]   # read these without asking
roots = ["~/notes"]                # also allow this directory
max_bytes = 16384
approval = "ask"                   # "deny" turns the tool off
```

Rules match the path as shown in the prompt (relative to the working directory) with `*` and `?`. Rules never cover credentials. `shellmate config check` warns about a rule such as `*` that matches every file.

## Why a read taints the request

File text is written by someone else. A README or a log can contain lines that try to instruct the model. shellmate labels file text as reference data, and **once a file's text is in the conversation, the rest of that request is tainted**: your `preapproved` rules for both `read` and `exec` stop applying, so anything the model asks for next needs your approval at the prompt. The same applies after a command's output comes back.

## Project and git context

Two small collectors add context without reading any file:

| Collector | Level | What the model learns |
|---|---|---|
| `project` | 1 and up | Which kinds of project the working directory is, from file **names** only: `Cargo.toml`, `package.json`, `go.mod`, `Makefile`, `Dockerfile` and similar. Contents are never read. |
| `git` | 2 and up | The branch, how far it is ahead of or behind its upstream, and counts of staged, modified and untracked files. |

```toml
[context]
project = true
git = true
git_log_lines = 0     # also send N recent commit subjects; see below
```

- Both only run when the provider's context level allows it, so a remote provider at level 1 gets the project line and no git state.
- **Commit subjects are off by default.** They are text other people wrote, so sending them taints the request. Set `git_log_lines` (up to 20) if you want the model to see recent commits.
- A branch name is shown only when it is plain (letters, digits and `._/+@#-`); anything else is replaced by "a branch with an unusual name".
- **Cloned repositories cannot run code through this.** A repository's own config can name a program for git to run when it checks the working tree (`core.fsmonitor`). shellmate turns that off for its own git calls and for any `git` command the exec tool runs, without changing the command you approve.
