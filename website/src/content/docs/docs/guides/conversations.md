---
title: Follow-up conversations
description: How shellmate remembers earlier turns in the same shell, where they are stored, and how to manage them.
---

Press the shortcut again and the model remembers what you asked before in this shell:

```text
$ how do I find big files?               ← alt+m
$ du -sh * | sort -rh | head
$ same, but only .log files              ← alt+m
$ ls -lhS *.log
```

## How it works

- Each interactive shell gets its own **session**. The integration exports it as `SHELLMATE_SESSION` and sends it with every request. Without a session (for example when you call the binary by hand) every request stands alone.
- A session has one **current conversation**. A conversation that has been idle for 30 minutes is replaced by a new one.
- Conversations are stored on **tmpfs** (`$XDG_RUNTIME_DIR`), so they are gone at logout and never reach your disk. Files are mode 0600 in 0700 directories, and a directory that is not private is refused.
- Only the most recent turns are replayed (6 by default). Older tool results are shortened.

## Not every turn is replayed everywhere

A turn that used your recent commands (context level 2 or higher) is **never replayed to a `trust = "remote"` provider**. When turns are left out, the model is told, and you see a note. Secrets keep the same placeholder in every turn of a conversation, so the model can still refer to them consistently without ever seeing them.

## Commands

```sh
shellmate conversation show       # what the model remembers, including tool calls
shellmate conversation undo       # forget the last turn
shellmate conversation new        # start over
shellmate conversation list       # stored conversations; the current one is marked
shellmate conversation resume ID  # make a stored conversation the current one
shellmate conversation clear      # delete the current one; add --all for every conversation
```

Bind a key to start over without typing a command: `"alt+k" = "new-conversation"`.

## Configuration

```toml
[conversation]
store = "builtin:store-tmpfs"   # "none" turns follow-ups off
idle_timeout = "30m"            # a number plus s, m, h or d
max_turns = 40                  # older turns are dropped from the store
max_tool_rounds = 8             # tool calls allowed per request
[conversation.history]
strategy = "window"
keep_recent_turns = 6           # turns replayed to the model
elide_tool_results_after = 2    # older tool output is shortened

[actions.explain]
conversation = "fresh"          # auto (default) | continue | fresh
idle_timeout = "5m"
```

`auto` continues the current conversation unless it has been idle for too long. `continue` always does, and `fresh` never does.

If the store cannot be used (wrong permissions, no runtime directory), conversations are switched off and you are told so; requests still work.
