# Copilot evaluation: completeness, accuracy, experience

Measured on the local `lakehouse-run` stack between 2026-09-26 and
2026-09-27, with the harness in `ops/ai_eval/` (23 cases). On MiniMax-M3 the
overall score went from **0.772 to 0.998**: completeness 0.882 to **1.000**,
accuracy 0.578 to **0.996**, with 22 of 23 answers perfect. The one
remaining flag is a real one-digit slip the checker caught.

A 4-billion-parameter model (`qwen3:4b`, CPU only) answered correctly
whenever it finished. After the fixes it made no wrong claims on the
cases re-run, but on a CPU it takes minutes per answer.

## How it is measured

`python3 ops/ai_eval/ai_eval.py --label <name>` asks `POST /api/ai/chat` 23
questions and scores each answer. The expected values are not written in
the cases: each one is a SQL query (or an API list call) run at evaluation
time against the same ClickHouse the API reads.

| Score | What it measures |
| --- | --- |
| Completeness | Share of expected facts present: numbers within a tolerance, names case-insensitively, and the expected tool call for the build case. |
| Accuracy | 0 when an answer that should decline does not, or when a forbidden pattern appears (raw database error, invented currency). Minus 0.1 per number the API's citation checker left unverified, down to 0.5. |
| Experience | Latency, and answering in the question's language. |
| Overall | 0.4 × completeness + 0.4 × accuracy + 0.2 × experience. |

The 23 cases cover:

- the layer overview;
- aggregates, rankings, shares, growth and peak months;
- a schema question;
- two questions the data cannot answer (a metric that does not exist, and a year outside the range);
- lineage and quality;
- failing pipelines, empty connector and alert lists, dashboards and CDC health;
- a two-turn follow-up;
- an Indonesian question;
- a build-mode chart request.

A matching number shows the answer printed the right value, not that the
sentence around it is right. Every score change below was also checked by
reading the saved answers.

## Results on MiniMax-M3

| Run | Completeness | Accuracy | Experience | Overall | Perfect cases | Unverified numbers | Median latency |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline (before) | 0.882 | 0.578 | 0.939 | 0.772 | 2 / 23 | 175 | 4.8 s |
| v1 | 0.949 | 0.843 | 0.948 | 0.907 | 11 / 23 | 16 | 4.0 s |
| v2 | 0.935 | 0.930 | 0.957 | 0.937 | 16 / 23 | 16 | 4.9 s |
| v3 | 0.957 | 0.991 | 0.965 | 0.972 | 19 / 23 | 2 | 4.0 s |
| v4 | 0.957 | 1.000 | 0.957 | 0.974 | 22 / 23 | 0 | 5.0 s |
| v7 | 0.935 | 0.978 | 0.991 | 0.963 | 20 / 23 | 5 | 4.8 s |
| **v8 (final)** | **1.000** | **0.996** | **1.000** | **0.998** | 22 / 23 | 1 | 4.0 s |

- **v4:** the case below 1.00 is a MiniMax HTTP 5xx; the retry landed after it.
- **v7 found two problems:**
  - An invented follow-up figure survived the repair round, because the model answered it without a tool call.
  - The repair round's own narration ("Final answer: …") leaked into an answer.
- **v8 fixed both.** Its one flag is a real mistake the checker caught: 2,643,888 printed as 2,648,888.
- **Re-scoring.** Scores after v4 use the harness's re-score fixes: "does not include" counts as declining, and a case with no expected facts is not scored as declining. `--rescore` re-scores saved answers; v4 re-scores to the same 0.974.

In the baseline, most of the 175 unverified numbers were correct values
the citation checker could not match (see "Citation checker" below). The
baseline's accuracy therefore understates the model and overstates the
false alarms users saw. Both were real problems for users.

## What was wrong, and what changed

| Found in the baseline | Cause | Change |
| --- | --- | --- |
| Correct figures shown as unverified; whole tables replaced with "[table omitted]" | `ClickHouse` returns every value as a string, and the checker only read JSON numbers; `**bold**` numbers were never parsed; text cells needed a word-for-word match | `citations.rs`: numeric strings are evidence, as are row counts, per-value counts and column totals. The DATA MAP and earlier turns count too. Bold, scale words, Indonesian number format, SI byte units, rank columns and label cells are handled. Arithmetic between two numbers of one small result is verified. |
| A wrong total (12,934 printed as 13,934) went out unflagged | The model added numbers itself, in bold | The prompt makes SQL do all arithmetic. The checker now parses bold numbers. |
| A follow-up printed an invented year total with no tool call | Nothing stopped an unbacked figure reaching the user | A **repair round**: when the final answer has unverified figures, the model gets them back once ("look them up with run_sql or remove them"). The console shows "Checking the figures…". |
| "No data source connectors are registered" arrived as "source connectors are registered" | MiniMax-M3 sometimes writes the first answer words inside `<think>`, and stripping the block deleted them (2 of 5 identical turns) | `lakehouse-llm`: a short fragment after the reasoning's last sentence break is put back when the answer starts mid-sentence or mid-number. |
| "What data do we have?" answered in primer/sekunder, not Bronze/Silver/Gold | The prompt never explained the layers; the tools only returned `tier` | New English, tenant-neutral prompt (`prompt.rs`). `sourceKind` replaces `tier`. The new read-only `lakehouse_overview` tool. `describe_dataset`/`get_lineage` report per-layer presence. |
| A year outside the data answered with a guess, and a 700-row result read as "only months 1–2 loaded" | No value ranges in the prompt; results cut at 8,000 characters mid-JSON | The DATA MAP (`data_map.rs`) lists columns, catalog descriptions, ranges and the stored text values for every Gold/Silver table (cached 120 s). Results are capped at 100 rows, with a note saying the data is complete and to aggregate in SQL. |
| English questions answered in Indonesian, and the reverse | The catalog metadata in the prompt is Indonesian | The reply language is detected server-side and named in the prompt's last line. |
| Raw `DB::Exception … _silver_meta does not exist` in answers and on the Data Quality page | Tools and `GET /api/governance/{kind}` returned upstream text | Classified messages ("no results yet: the job that records them has not run"). The same applies to `run_sql` errors (message kept for self-correction, boilerplate removed) and to the LLM-unavailable body. |
| `describe_dataset` reported `rows: 0` for a 720-row table | It counted a Silver table that did not exist | Per-layer `present: true/false`, never an invented 0. `describe_mart` reports an unknown count as null. |
| One model 5xx cost the user the whole answer | No retry | Transient failures (connection, 408, 429, 5xx) are retried twice (1.5 s, 4 s). A 4xx such as a bad key is not retried. |
| A small model's answer came back empty | Reasoning used the whole 1,200-token cap | 4,096 tokens per round, and one "answer now" nudge for an empty round. |
| A small model read one row as a total | The prompt never said a row is one combination of dimensions | Grain line per table in the DATA MAP ("one row per tahun x bulan_no x negara; SUM jumlah"), plus a rule in the prompt. |
| The repair round's narration reached the user | The model explained its correction | A preamble before a "Final answer:" line is dropped, and the repair request says to reply with the answer only. |
| Providers ignore forced tool choice | Tested: neither MiniMax nor Ollama honours `tool_choice: "required"` or a named function | The repair request tells the model to start with `run_sql`. A figure it still cannot back stays flagged. |

## Built for small models

Published work on tool calling and text-to-SQL points the same way for
small models:

- fewer tools per request (accuracy drops steeply past roughly 15–20 tools);
- column descriptions and sample values in the prompt;
- execution feedback so the model can fix its own SQL;
- explicit abstention when the data cannot answer.

The changes follow that:

- **Per-turn tool selection** (`prompt::select_tools`). A data question sees 12 tools instead of 23 (ask mode) or 48 (build mode). Other domains are added only when the conversation mentions them. The gate still decides what may run.
- **Plain numbered rules** in the prompt, one instruction per line, with the decisions spelled out ("compute in SQL", "say the data does not include it").
- **The DATA MAP carries stored values and ranges**, so the model never has to guess a literal or a range.
- **Compact tool results** and error messages the model can act on.
- **`AI_CHAT_TIMEOUT_SECS`** (default 120, clamped 30–1800) for a slow self-hosted model. One CPU-served turn takes minutes and would otherwise always time out.

## Small model: `qwen3:4b` (Ollama, CPU only)

`qwen3:4b` ran under Ollama on this machine's 8-core CPU with no GPU, with
`AI_CHAT_TIMEOUT_SECS=1800`. Latency is a property of this hardware, so it
is reported apart from correctness.

**Full run (v5, 23 cases, re-scored with the fixed harness).**

| Score | Value |
| --- | ---: |
| Completeness | 0.848 |
| Accuracy | 1.000 |
| Experience | 0.191 |
| Overall | 0.777 |

**Latency.** Median about 10 minutes per answer, range 100 s to 30 min.

**Where it was right.** 19 of 23 cases were complete and correct:

- the layer overview;
- totals, rankings, shares and growth;
- the Indonesian question, answered in Indonesian;
- the schema question;
- lineage and quality;
- the empty connector and alert lists;
- dashboards;
- the two-turn follow-up;
- both questions the data cannot answer, declined correctly;
- the build-mode chart request.

**What went wrong, and what was done about it:**

| Case | What happened | Fix, and the re-run on `qwen3:4b` |
| --- | --- | --- |
| Peak month | Returned one country's row as the month total (a real error, invisible to number matching: the value was in the result) | Each table's grain and "SUM the measure" added to the DATA MAP and prompt. Re-run: correct month and total. |
| Failing pipelines | Read the run log (`get_build_status`) and said nothing was failing | The two tool descriptions now say which question each answers. Re-run: used `list_pipelines`, answer matched the live state. |
| Two-part atlas question | Timed out at 30 min in both runs | None: CPU speed. |
| CDC health | True status and lag, but no slot name | `get_cdc_health` now returns each slot's latest check (40 samples became 1). The answer still omitted the name. Open. |

**Hidden failure: empty answers.** One earlier (v4) answer came back empty:
the model spent the whole 1,200-token output cap on reasoning. The cap is
now 4,096 per round, and an empty round gets one "answer now" nudge; no
empty answer occurred after that.

**Conclusion.** The mechanisms that raised the large model's accuracy (the
DATA MAP with stored values and grain, per-turn tool selection, the
citation check and repair round, plain rules) also carry a 4B model to
correct answers. For interactive use, serve it on a GPU or a hosted
small-model endpoint: on CPU it answers in minutes.

## Known gaps

- **Latency.** On CPU, a 4B model takes minutes per turn; that is a hardware
  limit, not something the prompt can fix. A GPU or a hosted small model is
  needed for interactive use.
- **Coverage of the eval.** The 23 cases run on one small demo dataset.
  They show the mechanisms work; they do not prove the same scores on a
  large warehouse. Add cases when a real deployment has new question
  shapes.
- **Repair round.** It is one extra model call, taken only when a draft
  has unbacked figures. The second answer is shown as it is, flags
  included; a model that ignores the request (no tool call) keeps its
  flags.
- **Number matching is not meaning checking.** A number that is in the
  tool result but answers the wrong question (one row read as a total)
  passes the citation check. The grain rule reduces this; reading answers
  is still part of evaluating a change.
- **Unrelated test failure.**
  `state::tests::connector_secret_resolver_admits_credential_suffixed_refs…`
  fails whenever the tests run in a checkout that has a local `.env` with
  `CONNECTOR_PG_PASSWORD`. sqlx's Postgres test harness calls
  `dotenvy::var`, which loads that file into the process. It passes in a
  checkout without `.env`, and it is unrelated to this work.
