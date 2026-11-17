# AI-24 T5 result — the stream through the console's own hop

`AI-24` (`docs/superpowers/plans/2026-11-02-ai-24-history-and-streaming.md`)
D9 asks one question that the code cannot answer by reading it: does the
console's `/api/:path*` rewrite (Next.js, `next.config.ts`) hold the answer
back until the end, or forward each NDJSON line as it arrives? The API writes
and flushes one line at a time (`routes/ai/mod.rs`), and the integration test
proves those lines reach the caller; the console's second hop was **not
verified**, and this is the measurement.

Measured on 2026-11-02, at commit `87716472` (branch
`fix/ai-24-history-and-streaming`), on the developer machine.

## Method

- **The upstream**: a small stub origin answering `POST /api/ai/chat` with the
  real route's NDJSON contract — one `{"type":"delta"}` line at once, then 50
  more 50 ms apart, then the `done` body — and the real route's headers
  (`content-type: application/x-ndjson`, `cache-control: no-cache,
  no-transform`, `x-accel-buffering: no`, the AI-24 D8 set). It stands in for
  the API process; the hop it is measured across is the one under test.
- **The console**: built from this branch with the rewrite pointed at the stub
  (`RUST_API_URL=http://127.0.0.1:$STUB_PORT bun run build`), then
  `PORT=3000 bun run start`. The rewrite target is fixed at build time, which
  is the reason for the rebuild.
- **The measurer**: one `fetch` with `stream: true`, reading the body with a
  `ReadableStream` reader and recording the arrival time of every chunk. The
  numbers are the time of the first chunk from before the request, the number
  of chunks the 51 lines arrived in, and the largest gap between two chunks.
- Each path was run three times, alternating nothing (the stub stayed up).

## Numbers

Direct to the stub origin (the API's own port in the real stack):

| Run | First chunk | Chunks | Lines | Largest gap | Total |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 7 ms | 51 | 52 | 51 ms | 2516 ms |
| 2 | 1 ms | 51 | 52 | 51 ms | 2512 ms |
| 3 | 1 ms | 51 | 52 | 53 ms | 2513 ms |

Through the console's rewrite (`:3000/api/ai/chat`):

| Run | First chunk | Chunks | Lines | Largest gap | Total |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 29 ms | 51 | 52 | 53 ms | 2536 ms |
| 2 | 4 ms | 51 | 52 | 50 ms | 2511 ms |
| 3 | 3 ms | 51 | 52 | 50 ms | 2511 ms |

## Conclusion

The console path does not batch. All 51 pieces arrived as 51 chunks on both
paths, the largest gap stayed at the stub's own 50 ms cadence (50–53 ms), and
the first piece through the console came 3–29 ms after the request — well
inside D9's 200 ms bound (the 29 ms run is the first request through a cold
proxy). The 52nd line (the `done` body) shared its chunk with the last delta
on both paths, which is a read boundary, not a buffered response: a buffered
response would have arrived as one chunk after 2.5 s.

D9's stop-and-report condition (fewer chunks, or the first piece more than
200 ms later than direct) did not occur. No change to Next.js compression is
needed by this measurement, and none was made.

## Not verified

- **The real API process was not run.** It needs Postgres, ClickHouse and a
  seeded session; standing that stack up was outside this task's time. The
  API's per-line write-and-flush is covered by
  `routes/ai/mod.rs`'s `stream_chat` and asserted end to end by
  `tests/ai_chat_stream.rs`, and the hop measured here is the one D9 names.
- **One real answer of about 1,500 characters through the console in a
  browser was not run.** A model endpoint is configured in this environment,
  but reaching it needs the full stack above plus a signed-in browser session;
  it was not measured. The reveal's pace and its end-of-stream handover are
  covered by `smooth-text.test.tsx` with controlled frames (a 300
  characters-a-second answer trails by ≤ 120 characters; a checked answer that
  starts with the shown text goes on one frame's step at a time), which is a
  rule tested against the code's own constants, not a recording of a model.
