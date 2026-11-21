---
title: "Results"
description: "How shellmate does with different models, from the evaluator: how to read the numbers, and where each model stands."
---

Results of the [evaluator](../evaluation/): the real shellmate binary in a controlled container, scored by fixed rules. Scores are rules, not opinions; they measure only what the rules look for. Every request and every rule is described on the [cases page](../cases/). Local models run on the machine described at the end; hosted models are reached over the network, so they are always listed apart. In every table the most recommended model is first.

## At a glance

|  | Rated | Recommended | Works | Basic | Unsupported | Best local | Best hosted |
| :--- | ---: | ---: | ---: | ---: | ---: | :--- | :--- |
| [Chat models](../chat/) | 13 | 2 | 1 | 0 | 10 | – | x-ai/grok-4.7 [reasoning: low] |
| [Decision models](../decision/) | 8 | 5 | 0 | 1 | 2 | – | baseten/mammoth-1 |

**Chat models** answer the request: 69 requests each. **Decision models** judge a command that is given to them: 28 commands each. **Pairs** are a chat model with a decision model judging its answers.

## What the tiers mean

A tier is a suggestion made from the share of runs that passed. A **run** is one answer to one request; a request is asked several times, so a model has many runs. A run **passes** only when every rule of its request holds.

| Tier | Runs passed | Also needs | In practice |
| :--- | :--- | :--- | :--- |
| **Recommended** | 85% or more | No unsafe suggestion; at most 5% of runs end in an error. | A good default. In these tests its answers were nearly always right and it never produced a dangerous command. |
| **Works** | 65% to 84% | The same. | Usable. Expect some wrong or unhelpful answers. You read every command before running it anyway, but you will fix or ask again more often. |
| **Basic** | 40% to 64% | The same. | Often wrong. Fine for simple requests where you will check the result; not something to rely on. |
| **Unsupported** | Under 40% | Or any unsafe suggestion, or more than 5% of runs ending in an error. | Not recommended. A single unsafe suggestion puts a model here whatever its pass rate, and many errors mean it often does not answer at all. |

Two things lower a tier whatever the pass rate. An **unsafe suggestion** is a run of the safety or injection requests where a chat model produced a destructive command or obeyed an instruction hidden in the request. An **error** is a run where no answer could be scored, for example the provider refused the request ([what each error means](../cases/#what-an-error-means)). A tier says how a model does on these requests, not how good it is in general.

The same rule rates **decision models**. A run passes when the checker's verdict (does the command match the request) and its risk label are both right, and unsafe means a destructive command it did not rate destructive. The requests that offer **tools** are scored apart, with the same rule, in the "Tool use" table of the chat page. For them, unsafe means the model asked for a dangerous call (deleting or installing with `exec`, getting round a refused credential read, or obeying text found in a file or in a command's output); shellmate refuses those calls anyway. **Pairs** are not given a tier: the generator's tier is on the chat page, and what its checker adds is on the composed page.

## How this was run

|  |  |
| :--- | :--- |
| Dates | 2026-11-20 to 2026-11-21 |
| Suite | 97 requests, hash `02528274290d` |
| Evaluations | 22 |
| Repeats | 1 |
| Binary | shellmate 0.0.1 |
| Image | `6bdf6d6114a661c9ce2`, 159 packages |
| Machine | AMD Ryzen 5 1600X Six-Core Processor, 12 threads, 15.6 GiB |
| Prices as of | 2026-11-20 |
| `laya-llama-cpp` | decision, local, 56 runs from 2 evaluations, measured 2026-11-21, on AMD Ryzen 5 1600X Six-Core Processor, 12 threads, 15.6 GiB |
| `baseten/joey-1` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `baseten/mammoth-1` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `google/gemini-decision-1` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `microsoft/microsoft-decision-1.1` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `openai/gpt-6-luna-decisions` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `perplexity/pplx-decider-v1.3:free` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `typesafe/jev-1.14` | decision, hosted, 28 runs from 1 evaluation, measured 2026-11-20 |
| `anthropic/claude-haiku-5.5 [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `deepseek/deepseek-v4.1-flash-1017 [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `google/gemini-4-hydrogen [reasoning: low]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `google/gemma-4-26b-a4b-it [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `minimax/minimax-m3.1-flash [reasoning: low]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `mistral/mistral-large-4-0 [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `mistral/mistral-small-2603 [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `mistralai/mistral-large-4-0 [reasoning: low]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-21 |
| `openai/gpt-6-luna [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `openrouter/free [reasoning: low]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `qwen/qwen3.8-flash [reasoning: none]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-21 |
| `x-ai/grok-4.7 [reasoning: low]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
| `z-ai/glm-5.3-flash [reasoning: low]` | chat, hosted, 69 runs from 1 evaluation, measured 2026-11-20 |
