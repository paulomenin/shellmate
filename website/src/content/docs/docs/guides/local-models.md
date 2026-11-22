---
title: Choosing a local model
description: Which small models run fast on your own machine and suit shellmate, with measurements and how to try them.
---

shellmate asks for short answers: usually one command and a sentence. That is easy work for a small model, and a local one keeps your shell history on your machine. What matters is **latency** (you are waiting at a prompt) and **getting the command right**, not raw intelligence.

:::note
Model lineups change quickly. This page was checked in November 2026 against the [Ollama library](https://ollama.com/library). Sizes and tags are what Ollama lists; the timings were measured by the author on one machine (below). Treat everything else as a starting point and test on your own requests.
:::

## What to look for

- **Fits in memory.** A model that does not fit in VRAM spills to the CPU and gets several times slower. The download size on the model page is a good approximation of the memory needed, plus some room for context.
- **Time to first word.** Under a second feels instant. Over a few seconds, you will stop using it.
- **Follows the output format.** shellmate asks for exactly one command in a fenced block. Models that ramble or add extra blocks give worse results.
- **Reasoning off.** "Thinking" models spend most of their time reasoning silently before the first word. For short shell answers, switch it off with `reasoning_effort = "none"` (see [Providers](../providers/)).
- **Native tool support is optional.** By default shellmate describes the tools in the prompt and the model writes a block, so a model does not need native tool support. A server that says a model takes tools natively is used that way (`tool_calling = "auto"`), and you can set `native` or `text` on the model. See [Tool calling](../../concepts/tool-calling/#native-and-text).
- **Keep it loaded.** The first request after a server loads a model can take a long time (see below). Keep the model loaded between requests with your server's setting for it (with Ollama, `OLLAMA_KEEP_ALIVE`, for example `1h`).

## Candidates

Sizes are the download sizes listed by Ollama. "Tested" means the author ran it with the prompts below; the others are reasonable candidates that have **not** been tested with shellmate.

| Model | Tag | Size | Context | Notes |
|---|---|---|---|---|
| Qwen2.5 Coder | `qwen2.5-coder:1.5b` | 986 MB | 32K | Tested. Extremely fast, but made mistakes. |
| Qwen2.5 Coder | `qwen2.5-coder:3b` | 1.9 GB | 32K | Not tested. A step up from 1.5B for similar speed. |
| Qwen2.5 Coder | `qwen2.5-coder:7b` | 4.7 GB | 32K | Not tested. A good default for 8 GB GPUs or CPU-only machines with 16 GB RAM. |
| Gemma 4 | `gemma4:e2b` | 4.6 to 7.5 GB | 128K | Not tested. The smaller Gemma 4. |
| Gemma 4 | `gemma4:e4b` | 6.6 to 9.5 GB | 128K | Tested. Accurate and quick once loaded. A thinking model: use `reasoning_effort = "none"`. |
| Phi-4 mini | `phi4-mini` | 2.5 GB | 128K | Not tested. A small footprint with 3.8B parameters. |
| Qwen3 | `qwen3:4b`, `qwen3:8b` | 2.5 GB, 5.2 GB | 256K, 40K | Not tested. A thinking model: use `reasoning_effort = "none"`. |
| Qwen3.5 | `qwen3.5:4b`, `qwen3.5:9b` | 3.3 to 4.0 GB, 6.6 to 7.6 GB | 256K | Not tested. A thinking model, newer than Qwen3. |

If you have a bigger GPU (16 GB or more), larger models such as `qwen2.5-coder:14b` or the medium Gemma 4 and Qwen 3.5 sizes will make fewer mistakes, but you are paying in speed for answers that were already short.

## What the author measured

Hardware: an NVIDIA GTX 1060 with 6 GB of VRAM (an old card), 15 GB RAM, Ollama on localhost. Each model was asked four requests through the same system prompt shellmate uses for `generate`, after a warm-up request. Times are seconds to the first word and to the end of the answer.

| Request | `qwen2.5-coder:1.5b` | `gemma4:e4b` (reasoning off) |
|---|---|---|
| list the five biggest files in this directory | 0.11 / 0.26, `ls -lh \| head -n 5` | 0.62 / 1.30, `ls -lhS \| head -n 6` |
| find all .log files modified in the last day | 0.11 / 0.25, `find . -name "*.log" -mtime -1` | 0.64 / 1.17, `find . -name "*.log" -mtime -1` |
| show which process is listening on port 8080 | 0.11 / 0.26, `lsof -i :8080` | 0.65 / 1.64, `sudo lsof -i :8080` |
| compress the folder photos into a tar.gz excluding .tmp files | 0.11 / 0.29, `tar -czvf photos.tar.gz --exclude=".tmp" photos/` | 0.62 / 1.33, `tar -czvf photos.tar.gz photos --exclude='*.tmp'` |

What this shows:

- **The 1.5B model is nearly instant but got two of the four wrong.** `ls -lh | head -n 5` does not sort by size, so it does not list the biggest files, and `--exclude=".tmp"` does not match `*.tmp`. Both look plausible, which is the dangerous kind of wrong. It is fine for simple, common commands.
- **`gemma4:e4b` got all four right** in about a second and a half, even on this old GPU. (`head -n 6` includes the `total` line; the `sudo` in the port example is more cautious than necessary.)
- **The first request after loading is slow.** Loading `gemma4:e4b` took 139 seconds here before the first answer. After that, it stayed fast. Raise `timeout_secs` in the provider settings, and keep the model loaded.
- Four requests is a tiny sample. It shows the shape of the trade-off, not a ranking.

## Try one

```sh
ollama pull qwen2.5-coder:7b
```

```toml
[providers.local]
trust = "local"
preset = "ollama"
timeout_secs = 180

[models.qwen]
provider = "local"
plugin = "builtin:openai-compatible"
model = "qwen2.5-coder:7b"
# for thinking models (gemma4, qwen3, qwen3.5):
# reasoning_effort = "none"

[llm]
chain = ["qwen"]
```

Check it with `shellmate config check`, then use `alt+i` on a few requests you actually make. `shellmate invoke --explain` prints the time to the first word, so you can compare models with your own numbers.

## Using a local model and a cloud model together

A small local model is a good first choice, with a larger remote one as the fallback when the local server is down. List them in order in [`[llm] chain`](../providers/#fallback-chains). Remember that a remote provider only receives the level of context you allow for it.
