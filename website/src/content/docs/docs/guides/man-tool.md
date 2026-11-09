---
title: The man tool
description: Let the model read a manual page before answering, safely.
---

When you ask about a command's options, a model that guesses can be wrong. The `man` tool lets it read the real manual page first.

## How it works

1. The model replies with a request to read a page, for example `man tar` or `man 5 crontab`.
2. shellmate **classifies** the request. Only `man [section] page` with a plain page name is allowed. Options, paths, extra arguments and any other program are rejected. The model is told about a rejection and answers without the page.
3. If it is allowed, shellmate runs `man` itself, strips overstrike and ANSI formatting, caps the size, and hands the text back to the model as **reference data, not instructions**.
4. The model gives its answer. Tool requests and results are hidden from the streamed output but are recorded in the conversation (`shellmate conversation show`).

A request is limited to `max_tool_rounds` rounds (8 by default), so a confused model cannot loop forever.

## Configuration

```toml
[tools.man]
enabled = true
approval = "allow"   # "allow" or "deny"
max_bytes = 8192     # longest page text sent to the model
```

Turn it off with `enabled = false`, or keep it enabled but refuse every call with `approval = "deny"`.

## Requirements

The `man` program must be installed (`man-db` or `mandoc`). `shellmate doctor` warns if it is not.

## Why it is safe

The tool is a fixed program with validated arguments, not a shell. Nothing the model writes is passed to a shell, a page name cannot contain a path or an option, and the page text is framed as untrusted data, because a manual page is content from the outside world. See [Security model](../../concepts/security-model/).

:::note
Running arbitrary commands (`exec`) is **not** available yet. When it arrives it will always ask you first. See [Status and roadmap](../../concepts/roadmap/).
:::
