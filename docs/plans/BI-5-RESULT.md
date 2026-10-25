# BI-5 result: what the engine does with the limits and with Stop

Date: 2026-10-24 to 2026-10-25. Plan: `docs/superpowers/plans/2026-10-24-bi-5-limits-and-stop.md`
(section 5 "Must verify"). Feature page: `docs/core/features/query-limits-and-stop.md`.

## What was measured and what was not

Measured: the engine's behaviour for the exact URL settings `ChClient::query_with_options`
sends, on a throwaway `clickhouse/clickhouse-server:26.8` container (version `26.8.9.10`,
2 GB, 1 CPU, the `default` user, no other load), driven with `curl` and Python `urllib` from
the machine itself. The slow statement was a single-thread scan,
`SELECT count() FROM numbers(100000000000) WHERE rand() % 7 = 0 FORMAT JSON`, which runs for
far longer than any test here (it was always ended by a limit, a kill or a client close).

**Not measured (not verified):** the full path of a click in the console through the Next.js
rewrite and the API to the engine. The API binary was not built on this machine (disk and
memory; AGENTS.md "never `cargo build -p lakehouse-api`"), no console was run, and the
plan's "five runs from Query Studio" were therefore not done as written. What was timed is
the engine part: from the `KILL QUERY` statement the Stop route sends to the query's absence
from `system.processes`. The 5-second target for the whole click is **neither met nor
failed**; the part after the engine (the route, the proxy, the browser) is unmeasured and has
to be taken at the owner's QA (`BI-5-AC2`).

The grant question (section 5) is also not verified: the container's `default` user holds
every grant. Whether the API's own `ClickHouse` user may `KILL QUERY` its own queries with a
`LIKE` on `query_id` needs the stack's user and was not tested (compose and init SQL are owned
elsewhere; if a grant is missing, Stop answers `{ stopped: n }` and the kill fails with a
logged error).

Scripts (kept outside the repo): `measure.py`, `e2b.py`, `e4.py`.

## Results

### E1: `max_execution_time` as a URL setting ends a buffered `FORMAT JSON` query

`max_execution_time=2`, `wait_end_of_query=1`: HTTP **408** after **2.06 s**, body
`Code: 159. DB::Exception: Timeout exceeded: elapsed 2000.340 ms, maximum: 2000.000 ms.
(TIMEOUT_EXCEEDED)`. So a timeout is an error status (not 500, not 200) when nothing was sent
yet; `ChError::Server` is built for any non-2xx and classified by the code.

### E2: `cancel_http_readonly_queries_on_client_close=1` does not cancel a buffered query by itself

The client (`curl --max-time 3`) closes the connection after 3 s; the query was still in
`system.processes` **40 s later in all three runs with the setting** (and in the one run
without it). Adding `wait_end_of_query=1` alone: same. The server only notices a closed socket
when it writes to it, and a `count()` aggregate writes nothing until it ends.

This **falsifies the plan's assumption** (section 5, first item, and feature decision 3: "a
query the API gives up on is cancelled on the engine too") for the buffered path. The setting
helps `query_stream`, which receives rows; it does not help here.

With **progress headers** (`send_progress_in_http_headers=1`,
`http_headers_progress_interval_ms=500`) the query was gone **0.52 s** after the client
closed (1.05 s together with `wait_end_of_query=1`). `query_with_options` now sends them
(the interval is `max(1000 ms, limit / 80)`, because hyper reads at most 100 response headers
and a 30-minute limit would otherwise send thousands).

### E3: Stop = `KILL QUERY WHERE query_id LIKE '<user>:<run>:%' ASYNC`

Five runs; the query was started with `query_id = "<uuid>:<uuid>:0"`, left running about 1 s,
then the kill was sent. Time from sending the kill to the query's absence from
`system.processes`:

| run | queries before | kill to gone (s) | the query's own answer |
| --- | --- | --- | --- |
| 1 | 1 | 0.031 | HTTP 500, `Code: 394 ... QUERY_WAS_CANCELLED` |
| 2 | 1 | 0.010 | same |
| 3 | 1 | 0.011 | same |
| 4 | 1 | 0.009 | same |
| 5 | 1 | 0.011 | same |

The `LIKE` form of `KILL QUERY ... ASYNC` is accepted (as `default`). The cancelled query
answers `QUERY_WAS_CANCELLED` (394), which the API turns into the tile sentence "Stopped".

### E4: closing the connection, with the settings the client now sends

`max_execution_time=120`, `wait_end_of_query=1`, progress headers every 1.5 s,
`max_result_rows=2001` (`break`), `cancel_http_readonly_queries_on_client_close=1`; the client
closed after 3 s. Seconds from the close to the query's absence, five runs:
**1.55, 3.02, 3.02, 3.05, 1.53**. The cancel-on-close latency is up to one progress interval;
for the 60-second default it is at most 1.5 s, for a 30-minute limit about 22 s (still bounded
by the limit itself).

### E5: a tile limit of 10 s, and a second finding

Same settings with `max_execution_time=10`: the answer arrived after **10.00 s**, but with
**HTTP 200** and the body `Code: 159. DB::Exception: Timeout exceeded ...`. Once progress
headers have gone out, the status line is already sent, so the error is in the body. The client
used to read a non-JSON `200` body as an empty result, which would have shown a timed-out tile
as an empty chart. `ChClient` now reads a body that is the engine's exception as a
`ChError::Server` (`body_is_exception`; a bare `Ok.` stays an empty result).

### E6: `max_result_rows` + `result_overflow_mode = 'break'` cut only the final result, not subqueries

Measured by the planner (BI-5 review S5, 2026-10-25) on the dev `ClickHouse` 26.8.9.10 with
`max_result_rows = 2001, result_overflow_mode = 'break'`:

| Statement | Answer | Whole? |
| --- | --- | --- |
| `SELECT sum(x), count() FROM (SELECT number x FROM numbers(10000))` | `49995000, 10000` | yes |
| a filter `x IN (SELECT ... 5000 rows)` | `5000` | yes |
| `count()` over a 100,000-row `GROUP BY` subquery | `100000` | yes |

Only the final result is cut. A tile that aggregates over a larger subquery gets the right
aggregate; a tile whose final result is longer than the cap gets its first rows and the
`limitHit` marker. (The per-kind cap of the review's B1 does not change this: it moves the
outer cap for point maps and pivots only.)

## Other items of section 5

- Does a browser abort reach the API through the Next rewrite (the handler future dropped)?
  **Not verified** (no console run). Stop does not depend on it: the cancel route kills by
  query id, and the console calls it before aborting.
- `max_execution_time` not overridden for mart statements: the builder's only trailing
  `SETTINGS` are `join_use_nulls = 1`, `prefer_column_name_to_alias = 1` and the SQL-source
  text (`max_result_rows`, `result_overflow_mode`, `max_execution_time`, swapped at run time
  for the request's numbers); `fields.rs` has one test statement with
  `max_rows_to_group_by`. None sets `max_execution_time` outside the SQL-source text.
- Role references: `role.id` is `UUID` (`0001_init.sql`), `query_limits_role.role_id` is a
  foreign key to it.

## Known limits recorded for the owner

- The SQL source preview in the editor keeps the builder's default text (60 s, 2,000 rows); it
  does not follow the setting yet.
- A tile cut at the map-point limit carries no `limitHit` marker (only a row cut does).
- Stopping a query on Trino is out of scope (plan section 4).
