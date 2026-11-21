---
title: Security model
description: What shellmate protects, what it does not, and the rules the code enforces.
---

A tool that sends your shell to an LLM and suggests commands deserves a clear threat model. This is the short version.

## The model is not trusted

The language model can be wrong, can be manipulated by text it reads, and can hallucinate. shellmate never lets it decide anything security-relevant. These are the rules the code enforces, not hopes about the model's behavior:

1. **Nothing the model produces is executed without you (INV-1).** Suggested commands go into your input line and you press Enter. When the model asks to run a command with the `exec` tool, you approve it at a prompt only a human can answer, unless you wrote a rule for it yourself, and a classifier can overrule your rule. See [Letting the model run commands](../../guides/exec-tool/).
2. **Tools are validated, behind a classifier and a policy.** `man` accepts only `man [section] page`. `help` accepts only a program name and plain subcommand words and always adds `--help` itself. `exec` accepts only one simple command, run directly and never through a shell.
3. **Remote providers need redaction (INV-2).** A `trust = "remote"` provider is refused unless a redaction layer is active.
   The one thing that leaves outside that chain is the connection check of `setup`, which asks a server for its list of models. It carries no prompt, context or command, only the key, goes only to the address you just gave, and in interactive setup only after you agreed to use a remote service.
4. **Untrusted content is framed as data.** Manual pages and other tool output are given to the model as reference data, never as instructions.

## Threats and mitigations

| Threat | Mitigation |
|---|---|
| Prompt injection through a manual page, file or command output | Untrusted-data framing. Nothing runs without your approval, and the prompt shows the exact command. The worst case is a misleading answer or suggestion, which you still read before pressing Enter. |
| A destructive command suggested | Buffer-first design; you review it. |
| Secrets leak to a provider | Context levels, reversible redaction that fails closed, `--dry-run`, local providers, and secrets that are never stored in the config. |
| Secrets in logs or files | The audit log is content-free. Conversations live on tmpfs with mode 0600. Config holds only references. |
| Terminal escape injection from model output | Control, CSI and OSC sequences are shown as text, never obeyed, so an answer cannot set the title, move the cursor or write to the clipboard. This holds with colour and markdown on: only shellmate's own styles are written, and a test over generated input checks it. |
| Tampered config | No project-local config is ever loaded. Protect `~/.config/shellmate` like `~/.ssh`. |
| Binary hijack through `PATH` | The generated integration embeds the absolute path of the binary. |
| Insecure endpoints | Certificates are always validated. Cleartext `http://` is accepted only for loopback hosts. |
| Runaway cost or loops | `max_tool_rounds`, output caps and request timeouts. |

## Secrets

Secret values are never written in the config. They are references: `env:NAME`, `file:/path`, `keyring:service/account` or `cmd:program args`. A `file:` secret must be mode 0600 or tighter. The loader rejects inline values, `config show` prints only the references, and `doctor` checks that they resolve without printing them.

A `cmd:` reference runs a program from your own config file to get the secret, so it is limited in the same way the exec tool is: the program is started directly and never through a shell, the line may contain no shell syntax, a relative program path is refused, and the program gets a small environment, no input, a timeout and capped output. Its output is the secret and is never printed. A `keyring:` reference is short for the `cmd:` line of `secret-tool` or `security`, so the same limits apply. The line is read only from your config (there is no project-local config), and nothing a model produces can set it. The program runs on first use of its provider, not at start-up, so an unused fallback provider never runs it.

## What is out of scope

shellmate does not defend against a compromised user account or root, a malicious terminal emulator, a compromised operating system, or a provider that breaks its own policy beyond what the trust levels limit. Redaction is best effort: it cannot recognize a secret format it has never seen.

## Reading files

The [read tool](../../guides/read-tool/) is limited to the working directory (and roots you choose), resolves symlinks before checking, never reads credential-looking files whatever your rules say, and asks before every file unless you wrote a rule. Git is run with a repository's own program hooks (`core.fsmonitor`) turned off, both for context and for approved `git` commands, because a cloned repository could otherwise execute code when git checks its working tree.

## Command checks

The [validator](../../guides/validator/) only adds notes. It can never approve or run anything, and a decision model's answer cannot loosen the classifier. A remote decider follows the same rule as a remote provider: it is refused unless redaction is active, and it receives only your request, the suggested command and your shell's name.

## Tainted requests

A request is **tainted** when it carries content a program or a remote party wrote, and a tainted request suspends your `exec`, `help` and `read` pre-approvals (INV-4). That covers output you captured with `shellmate run`, the text of a file the model read, the output of a command it ran, the help text of a program, and commit subjects when you enable them. From the moment such content enters a request, `read`, `help` and `exec` pre-approvals stop applying to it. Web search and MCP results will taint a turn the same way once those tools exist. See [Status and roadmap](../roadmap/).

## Reporting a vulnerability

Please report security problems privately, using GitHub security advisories on the [repository](https://github.com/paulomenin/shellmate).
