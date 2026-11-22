---
title: "Composed"
description: "What a decision model adds to a chat model: how often it flags the answers that are wrong, and the ones that are right."
---

A chat model's answers, judged by a decision model. Local and hosted pairs are listed apart. The chat model's own results are on the [chat page](../chat/) and the checker's on the [decision page](../decision/).

## Does the checker help?

A generator answers and a checker then judges its command. The checker is advisory, as in the application, so it cannot make an answer better; what it can do is warn. A checker **flags** an answer when it says the command does not match the request (a match probability of 20% or less, where shellmate warns) or rates it destructive. A useful checker flags wrong answers and leaves right ones alone. Pairs are ordered by how much more often they flag wrong answers than right ones.

### Hosted models

| Generator + checker | Wrong answers flagged | Unsafe answers flagged | Right answers flagged | Decision returned | Checker time p50 | Cost USD |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| **deepseek/deepseek-v4.1-flash-1017 [reasoning: low] + baseten/mammoth-1** | 3/4 (75%) | 1/1 (100%) | 1/31 (3%) | 100% | 537 ms | 0.0008332 |
| **deepseek/deepseek-v4.1-flash-1017 [reasoning: low] + perplexity/pplx-decider-v1.3:free** | 3/4 (75%) | 1/1 (100%) | 2/31 (6%) | 100% | 536 ms | 0 |
| **openai/gpt-6-luna [reasoning: none] + google/gemini-decision-1** | 3/4 (75%) | 2/2 (100%) | 2/29 (7%) | 100% | 700 ms | 0.0007392 |
| **deepseek/deepseek-v4.1-flash-1017 [reasoning: low] + google/gemini-decision-1** | 3/4 (75%) | 1/1 (100%) | 3/31 (10%) | 100% | 666 ms | 0.0007824 |
| **openai/gpt-6-luna [reasoning: none] + perplexity/pplx-decider-v1.3:free** | 2/4 (50%) | 1/2 (50%) | 2/29 (7%) | 100% | 514 ms | 0 |
| **openai/gpt-6-luna [reasoning: none] + baseten/mammoth-1** | 1/4 (25%) | 1/2 (50%) | 1/29 (3%) | 100% | 546 ms | 0.0007878 |

**Columns**

- **Wrong answers flagged**: of the answers with a command that broke any rule of their request (unsafe ones included), how many the checker flagged. Higher is better.
- **Unsafe answers flagged**: of the answers that failed the `safe` check, how many it flagged. The ones that matter most, and often few, so read the counts.
- **Right answers flagged**: of the answers that broke no rule, how many it flagged anyway: false alarms. Lower is better.
- **Decision returned**: of the answers with a command, the share where the checker gave a readable decision.
- **Checker time p50**: median time of the checker's call.
- **Cost USD**: the checker's requests, plus the generator's when the pair was run live; a pair judged on recorded answers costs only the checker's.

