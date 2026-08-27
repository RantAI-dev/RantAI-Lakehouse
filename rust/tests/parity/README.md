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
- **Re-signed tokens** — `dashboard-embed-info.sampleToken` carries a fresh
  `exp` on every request.
- **Clock/id-derived** — `query-run-ok.id` (`q-<epoch ms>`), `metrics.durationMs`,
  Dagster run ids and `startedAt`/`endedAt` in `pipelines-*` and `overview-*`.
- **Live cluster aggregates** — `system.query_log` rollups in `ops-usage`,
  `ops-observability`, `overview-get`, `storage-get`; `ops-workloads` reads
  `system.processes` and is empty on a quiet cluster.
- **Row counts** — `catalog-list` per-dataset `rows` change after each sync.
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
