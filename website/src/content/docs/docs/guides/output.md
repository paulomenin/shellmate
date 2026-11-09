---
title: Colours and formatting
description: How shellmate colours its output and formats the model's markdown, when it falls back to plain text, and how to change it.
---

shellmate shows its output in colour and formats the markdown in answers, when your terminal can show it. When it cannot, you get plain text, exactly as written.

## What you get

- **Answers** are formatted: `**bold**`, `*italic*` and `` `code` `` get their own look and lose their markers, lists get bullets and a hanging indent, headings and quotes stand out, and a fenced code block is shown with a bar down its side. Long lines wrap to your terminal's width.
- **Tool activity** while a request runs: a command that ran is green (yellow if it failed), a refusal or denial is yellow, a command moved to your prompt is cyan, and the first lines of its output are dim.
- **Errors** are red, with the hint on the next line. Findings under an answer (`check:`) and problems around it (a turn that could not be saved) are yellow, and the other commands you are offered are shown as code.
- **The waiting indicator** changes colour as it spins.
- **The other commands** use the same colours when they print to a terminal: `check` (red for a command that does not parse or is high risk, yellow for a missing program or medium risk, green for no findings), `config check`, `config show`, `context` (what is sent is green, what is held back is dim), `audit show`, `conversation` and `setup`. `check --json` and `init` are never styled.
- **`doctor`** marks each check `OK` (green), `WARN` (yellow) or `FAIL` (red), and the approval prompt colours the risk and the keys.

The colours are the 16 basic terminal colours, so they follow your terminal's own theme. The command that goes into your input line and the command in an approval prompt are never styled: you read exactly what will run.

## When it falls back to plain text

shellmate decides once per run, from the first rule that applies:

| Rule | Result |
|---|---|
| `[ui] color = "never"` | Plain text. |
| `[ui] color = "always"` | Colour, even when output is not a terminal. |
| `NO_COLOR` set to anything but empty | Bold, italic and underline, but no colour. |
| `CLICOLOR_FORCE` set (and not `0`) | Colour, even when output is not a terminal. `NO_COLOR` wins over it. |
| No terminal, or `TERM` is unset or `dumb` | Plain text. |
| `CLICOLOR=0` | Bold, italic and underline, but no colour. |
| Anything else | Colour. |

In plain text the model's markdown is left as written (`**bold**` stays `**bold**`). Symbols fall back to ASCII (`+`, `x`, `->`, `-`) when your locale is not UTF-8.

## Settings

```toml
[ui]
color = "auto"     # auto | always | never
markdown = true    # format the markdown in answers (only where colour or styling is on)
```

Set `markdown = false` to keep the colours but see the text as the model wrote it.

## Preview

`shellmate render` shows a markdown file the way an answer would look, so you can try your terminal and settings:

```sh
shellmate render notes.md --color 16 --width 60
shellmate render notes.md --color mono        # what NO_COLOR gives
shellmate render notes.md --color plain       # the fallback
cat notes.md | shellmate render --ascii
```

## What is formatted

Headings (`#`), bullet lists (`-`, `*`, `+`) and numbered lists (nested by indentation), quotes (`>`), rules (`---`), fenced code blocks, and inline `**strong**`, `*emphasis*`, `_emphasis_` and `` `code` ``. Everything else, including tables and links, is shown as written. A marker that never closes is shown as written too (`2 * 3`, `*.txt`).

## Safety

Text from the model, from tools and from your commands is escaped before it is drawn, whatever the colour setting: an answer cannot add its own colours, move the cursor or set the window title. The only escape sequences shellmate writes are its own styles.
