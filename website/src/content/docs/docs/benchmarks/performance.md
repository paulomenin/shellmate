---
title: Performance
description: How long shellmate itself takes, measured on AMD Ryzen 5 1600X Six-Core Processor.
---

How long the shellmate process takes on its own: start-up, building a request, checking the answer and drawing it. The model is replaced by a fake that answers instantly, so none of this includes waiting for a real model. Each scenario ran 100 times after 5 warm-up rounds, interleaved so that drift on the machine affects all of them alike.

## Whole process, wall time

| Scenario | What it measures | p50 | p95 | min | max | Above a bare process |
| :--- | :--- | ---: | ---: | ---: | ---: | ---: |
| `floor` | the system's cost of starting a trivial process (`true`) | 0.7 ms | 1.0 ms | 0.5 ms | 1.0 ms | – |
| `version` | `shellmate --version`: binary load and argument parsing | 1.6 ms | 2.4 ms | 1.4 ms | 2.6 ms | 0.9 ms |
| `config-check` | `config check`: load and validate the config, build providers | 5.2 ms | 8.3 ms | 4.7 ms | 8.7 ms | 4.5 ms |
| `dry-run` | `invoke --dry-run`: config, context, redaction and the request payload | 11.6 ms | 15.0 ms | 10.9 ms | 16.2 ms | 10.9 ms |
| `context` | `context`: the collectors (project files, git) in a small repository | 11.2 ms | 14.4 ms | 10.7 ms | 16.6 ms | 10.5 ms |
| `check` | `check`: syntax check in bash, program lookup, risk | 3.6 ms | 4.8 ms | 3.2 ms | 5.4 ms | 2.9 ms |
| `capture` | `invoke` with the capture action: wrap the line, no model | 1.9 ms | 2.8 ms | 1.6 ms | 3.0 ms | 1.2 ms |
| `invoke-ask` | a whole `ask` against an instant fake model, level 2 context | 14.7 ms | 18.1 ms | 13.7 ms | 18.9 ms | 14.0 ms |
| `invoke-generate` | a whole `generate` against an instant fake model | 14.6 ms | 17.9 ms | 13.8 ms | 18.7 ms | 13.9 ms |
| `invoke-stream` | a long (2 KB) streamed answer in small pieces: the renderer under load | 21.6 ms | 32.7 ms | 20.4 ms | 63.8 ms | 20.9 ms |
| `run-wrap` | `run -- true`: the cost of capturing a command's output | 2.7 ms | 3.6 ms | 2.3 ms | 3.9 ms | 2.0 ms |

"Above a bare process" subtracts the cost of starting `true` on this machine, which is what the operating system charges for any program.

## Where the time goes

Median per stage, from `invoke --timings=json`. "Model" is the fake, so it is the cost of the HTTP exchange and streaming, not of thinking.

| Scenario | Start-up | Context | Model (fake) | Checks | Overhead | Total |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| `invoke-ask` | 3.4 ms | 5.7 ms | 3.1 ms | 1.7 ms | 0.0 ms | 12.5 ms |
| `invoke-generate` | 3.4 ms | 5.7 ms | 3.1 ms | 1.7 ms | 0.0 ms | 12.4 ms |
| `invoke-stream` | 3.4 ms | 5.6 ms | 10.1 ms | 1.7 ms | 0.0 ms | 19.4 ms |

## Peak memory

Resident memory of the process at its largest, sampled on a few extra runs.

| Scenario | Typical | Worst |
| :--- | ---: | ---: |
| `version` | 2.1 MiB | 4.0 MiB |
| `config-check` | 8.1 MiB | 8.2 MiB |
| `dry-run` | 8.5 MiB | 8.6 MiB |
| `context` | 8.6 MiB | 8.8 MiB |
| `check` | 5.0 MiB | 5.2 MiB |
| `capture` | 4.2 MiB | 5.1 MiB |
| `invoke-ask` | 8.9 MiB | 9.0 MiB |
| `invoke-generate` | 8.9 MiB | 8.9 MiB |
| `invoke-stream` | 9.0 MiB | 9.0 MiB |
| `run-wrap` | 4.9 MiB | 5.5 MiB |

## Environment

|  |  |
| :--- | :--- |
| Date | 2026-11-03 14:16:14 -0300 |
| Version | shellmate 0.0.1 (commit 7b59690, with local changes) |
| CPU | AMD Ryzen 5 1600X Six-Core Processor (12 cores) |
| Memory | 15.6 GiB |
| OS | Linux 7.2.11-arch1-1 |
| Binary size | 6.5 MiB |
