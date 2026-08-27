# Parity corpus

Golden HTTP responses captured from the TypeScript backend before the Rust
cutover. `tests/parity.rs` replays these against the Rust service and asserts
structural equality. Because the cutover is big-bang, this corpus is the only
thing standing between a porting mistake and production.

## Capturing

```bash
bun --bun next dev &                       # needs .env.local + live ClickHouse
PUBLIC_DASH_TOKEN=<token> bun run rust/tests/parity/capture.ts http://localhost:3000
./rust/tests/parity/check-no-secrets.sh    # must pass before committing
```

## Sanitization

These responses come from a live system and are committed to git, so
`capture.ts` strips sensitive values **on every run**. This is deliberately in
code rather than applied by hand: a hand-sanitized corpus re-leaks the moment
somebody re-captures. Three layers:

1. **Known secrets** (`SECRETS`) — resolved from the environment for the
   outbound request, swapped back to `__PLACEHOLDER__` before anything is
   written. Add new ones here, never inline in `requests.json`.
2. **Credential shapes** (`SECRET_SHAPES`) — anything *looking* like a
   credential is replaced whether or not we knew it existed: `p_`+32-hex public
   tokens, bare 64-hex keys, and JWTs. This layer exists because the first real
   leak was the embed HMAC signing secret returned by `/api/dashboard/embed-info`
   — a value that was on nobody's list.
3. **Known credential keys** (`REDACT_KEYS_ALWAYS`) — `secret`, `sampleToken`,
   and friends are redacted by NAME, everywhere, regardless of shape. This is
   the primary defence for `/api/dashboard/embed-info`: `EMBED_SECRET` may be
   operator-supplied as uppercase hex, base64, or a passphrase, and layer 2
   would match none of those.
4. **Sensitive free text** (`REDACT_TEXT_IN` × `REDACT_TEXT_KEYS`) — real chat
   sessions and model prose become `<redacted:N>` length markers, preserving
   structure.

**Redaction is for sensitivity; normalization is for non-determinism.** These
are separate problems with separate owners, and conflating them cost real
assertion coverage twice during setup. A field that merely *varies* per call
(generated SQL, timestamps, run ids) stays in the corpus and is normalized by
the harness. Only what must not be persisted gets redacted. Concretely: `sql`
and `question` are NOT redacted — public business SQL and our own synthetic
test input — while `content` and `answer` are.

`check-no-secrets.sh` matches on shape, not on a known-value list, covering
hex secrets (any case), JWTs, provider keys, webhook URLs, credentials in URLs,
and email addresses. It runs in CI as the `parity-corpus-secrets` job and
should be run locally before committing any re-capture.

## Known non-deterministic captures

The harness must normalize these rather than compare them by value:

- **Model output** — `ai-chat-ok`, `agent-ask-ok`, `agent-query-ok`,
  `agent-text-to-sql-ok`. Redacted, but even the lengths vary per call.
  `tests/parity.rs` compares these four by top-level key presence + JSON
  *type* only (`STRUCTURE_ONLY`) — not even normalized leaf-by-leaf — because
  a different model run can legitimately call a different number of tools or
  return a different number of results, which a fixed-shape leaf normalizer
  can't tolerate.
- **Re-signed tokens** — `dashboard-embed-info.sampleToken` carries a fresh
  `exp` on every request; `dashboard-embed-info.secret` (the embed HMAC
  signing secret) is redacted at capture time (`REDACT_KEYS_ALWAYS`) and so
  can never be compared against a live value either.
- **Clock/id-derived** — `query-run-ok.id` (`q-<epoch ms>`), `metrics.durationMs`,
  Dagster run ids and `startedAt`/`endedAt` in `pipelines-*` and `overview-*`,
  `ops-workloads.workloads[].elapsedMs` (an in-flight query's live elapsed
  time).
- **Live cluster aggregates** — `system.query_log` rollups in `ops-usage`,
  `ops-observability`, `overview-get`, `storage-get`; `ops-workloads` reads
  `system.processes` and is empty on a quiet cluster. Also normalized for the
  same reason: `ops-observability.slos[].current` (a live SLO snapshot
  embedded in a string, e.g. `"1233ms"`, so the generic numeric-leaf rule
  can't catch it) and `ops-usage.tenants[].computeUnits`/`.budgetSpent` (a
  per-tenant echo of the same live `computeUnits7d` aggregate).
- **Row counts** — `catalog-list` per-dataset `rows` change after each sync.
- **Unordered catalog listing** — `catalog-list`'s `assets` and `namespaces`
  come from `SELECT ... FROM bronze_meta.dataset_catalog UNION ALL SELECT
  ... FROM bronze_meta_sec.dataset_catalog` with no `ORDER BY`, in both the
  TS and Rust backends. Confirmed live: three consecutive requests against
  the same unmodified Rust process returned two different row orderings.
  This is pre-existing, shared behavior (the TS route runs the identical
  unordered SQL), not a porting regression, so the harness sorts both sides
  by `id` before comparing rather than the SQL being changed to add an
  ordering guarantee neither backend ever had.
- **Session free text and tool-call SQL** — `ai-sessions-list.sessions[].title`
  and `ai-sessions-detail.session.{title,messages[].content,
  messages[].tools[].args.sql}` are redacted at capture time
  (`REDACT_TEXT_IN` × `REDACT_TEXT_KEYS` in `capture.ts`). Note: `sql` here
  genuinely is redacted in this specific capture, even though the "sensitive
  free text" section above says `sql` is not redacted in general (that
  statement is about the top-level `sql` field on `query-run-ok` /
  `agent-query-ok` / `agent-text-to-sql-ok`, which is real, intentionally
  unredacted business SQL — the tool-call SQL nested inside a stored chat
  session is a different field entirely and was swept up by the same-name
  free-text pass).
- **Runtime error text** — `alerts-create-bad-body` and
  `dashboard-boards-create-bad-body` record Bun's JSON parser message
  (`JSON Parse error: Unexpected identifier "not"`), which a Rust service
  cannot reproduce. Normalize the `error` field for these two.

## Deliberate omissions

- `/api/alerts/run` (GET + POST) is not captured at all. Its only guard is
  `ALERTS_RUN_TOKEN`, which is unset in this environment, so *any* request —
  including one with a deliberately bad token — evaluates every alert rule and
  can fire real webhooks and emails. There is no safe error path to capture.
- Success paths of all mutating handlers. Only validation/error paths are
  captured for POST/PUT/DELETE on `alerts`, `dashboard/specs`,
  `dashboard/boards`, `ai/sessions`, and `pipelines/{id}/trigger`.

## Real defects the harness caught (Tasks 1.13/1.14 first run)

Driving `tests/parity.rs` to 100% against a live Rust service (see "Running
the harness" below) found and fixed five real porting defects, none of which
were about volatile-field noise:

1. **`LayoutMap` used `std::collections::HashMap`** (`lakehouse-bi::store`).
   `/api/dashboard/export`'s hand-rolled YAML iterates a board's tile layout
   directly, and `dashboard-boards-list`/`dashboard-export` both capture a
   specific, non-alphabetical key order straight from the `layout_json`
   column. A `HashMap` made that order effectively random per process. Fixed
   by switching `LayoutMap` to `indexmap::IndexMap` (order-preserving on both
   JSON deserialize and iteration); `indexmap` is now an explicit workspace
   dependency (it was already pulled in transitively by `serde_json`'s
   `preserve_order` feature).
2. **`yaml_chart`'s field order followed `ChartInput`'s Rust struct
   declaration order**, not the `TypeScript` object-literal construction
   order in `bi-store.ts::specFromInput`. The two differ per chart `kind`
   (`mart` comes before `kind` for charts/tables, after it for
   text/kpi/gauge; `caption`/`target` sit before `span`/`board`, not after).
   Fixed by rendering `def` fields in three explicit, kind-scoped orders
   (`dashboard.rs::chart_def_fields`) instead of iterating
   `serde_json::to_value(def)`.
3. **Whole-valued `f64` fields serialized with a trailing `.0`**
   (`target: 3000000.0`, `queryErrorRate: 0.0`) where the TS backend's plain
   JS numbers render bare (`3000000`, `0`). Fixed at the single choke point
   every JSON API response passes through — `ApiJson::into_response`
   (`lakehouse-api::json`) — by converting the response to a
   `serde_json::Value` and rewriting any float-typed, whole-valued number to
   an integer before serializing, rather than annotating every affected
   `f64` field individually (`target` on `ChartSpec`/`ChartInput` also got a
   belt-and-suspenders `serialize_with`, since it feeds the manually-built
   YAML export too, which bypasses `ApiJson` entirely).
4. **`PUT /api/dashboard/specs` checked `id` in the wrong order.** The TS
   handler parses the body as loose JSON, checks `id` first, and only then
   builds/validates a `ChartInput` from it (coercing missing fields with
   `??`). The Rust handler deserialized straight into a `#[serde(flatten)]
   ChartInput`, so a body missing required `ChartInput` fields (and also
   missing `id`) failed the strict decode before ever reaching the `id`
   check — reporting `"body JSON tidak valid"` instead of `"id wajib untuk
   edit"` (exactly what `dashboard-specs-edit-missing-id` sends:
   `{"title":"x"}`). Fixed by parsing to a bare `Value` first and checking
   `id` before the strict `ChartInput` decode.
5. **`catalog-list`'s `assets`/`namespaces` have no guaranteed order** — see
   "Unordered catalog listing" above. Not a Rust defect (the TS route runs
   the same unordered SQL), but real enough to make the harness itself flaky
   until the comparison was made order-independent for those two fields.

## Running the harness

This test needs a live `lakehouse-api` process — it deliberately does not
spawn one itself, so a failure points at a real, inspectable process:

```bash
set -a; . ../.env.local; set +a
cargo run -p lakehouse-api &
PARITY_TARGET=http://localhost:8080 \
  cargo test -p lakehouse-api --test parity -- --ignored --nocapture
```

Set `PUBLIC_DASH_TOKEN` (same as the capture command above) to also exercise
`public-dash-ok`; without it, that one entry is skipped with a clear message
and every other entry still runs. `_OMITTED_alerts-run` is never replayed —
it isn't in the corpus at all (see "Deliberate omissions").
