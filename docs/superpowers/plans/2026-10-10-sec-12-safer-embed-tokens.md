# SEC-12 Safer embed tokens — Implementation Plan

**Status:** decisions signed 2026-10-10, not started. Written by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/safer-embed-tokens.md`. Spec:
`docs/core/specs/sec-12.md`.

**Base:** branch `fix/sec-phase-0` at `8b67ee2`. `SEC-11` is on it: every
error body goes through `upstream_error` or is our own fixed text, and
`tests/sec11_guard.rs` must keep passing.

**Goal:** signed embed tokens always expire, can be withdrawn, are signed
with a secret that is configured rather than invented, and the embed pages
can only be framed by sites the dashboard's owner listed.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Required claims | `exp` and `iat`, both numbers (Unix seconds). Missing or non-numeric: refused. |
| Lifetime | `exp − iat` ≤ the maximum; `iat` not in the future and `exp` not in the past, each with 60 seconds of tolerance. Maximum from `EMBED_TOKEN_MAX_LIFETIME_SECONDS`, default `86400`; a value that is not a positive integer stops the API at start with a config error. |
| No grace | Old tokens are refused at once. No setting re-admits them. |
| Withdraw all | Per dashboard: a "revoked before" instant stored with the board. A token whose `iat` is at or before it is refused. |
| Withdraw one | Optional `jti` claim (string, 1–128 chars of `[A-Za-z0-9._-]`). A withdrawn `jti` is refused for that dashboard until the token's own `exp` has passed, after which the record may be dropped. A token without `jti` cannot be withdrawn singly; say so. |
| Effect time | A withdrawal is honoured on the next request, and in any case within 60 seconds (no cache longer than that anywhere on the path). |
| Secret | `EMBED_SECRET` only. Remove the generate-and-store fallback in `lakehouse-embed` (`console.app_kv`). Unset or empty: signed embedding is unavailable; the API still starts. Compose keeps `${EMBED_SECRET:-}`. Do not delete an existing `embed_secret` row from `console.app_kv`; say in `docs/OPERATIONS.md` that it is no longer read and should be removed by the operator. |
| Framing | Per dashboard, a list of allowed origins (`https://host[:port]`, or `http://` for `localhost` only; at most 20; validated). The embed pages answer with `Content-Security-Policy: frame-ancestors <origins>`; an empty list is `frame-ancestors 'none'`. |
| Who | Withdrawing and editing allowed origins need `dashboard:write`, like the existing embed toggle. Reading them needs `dashboard:read`. |
| Messages | Ours and fixed: "embed token is invalid or expired" for every token refusal (one message, so a caller cannot tell which check failed), "embedding is not configured" when the secret is unset. |
| Out of scope | Public share links (`/api/public/dashboard/{token}`), which are not signed tokens. Their behaviour must not change. |

## 2. What exists today (anchors, verified at `8b67ee2`)

- `rust/crates/lakehouse-embed/src/lib.rs`: `EmbedClaims { resource, params, exp: Option<f64> }`; `verify_embed` accepts a token with no `exp`; the secret source prefers `EMBED_SECRET` and otherwise reads or generates `console.app_kv` `embed_secret` in plain text (`:160`–`:240`).
- `rust/crates/lakehouse-api/src/routes/embed.rs`: `POST /api/embed/data` (`Policy::Public`) verifies the token, loads the board, requires `embed_enabled`, merges the token's locked `params` into the filters, renders. `GET /api/public/dashboard/{token}` is the public share link.
- `rust/crates/lakehouse-api/src/routes/dashboard.rs:1333`–`:1401` `embed-info`: returns `enabled` and a one-hour `sampleToken`; the embed toggle is written through `PUT /api/dashboard/boards` (`:673`).
- Boards live in `ClickHouse` (`console.bi_board`, `lakehouse-bi` `store.rs`), with `embed_enabled` and `public_token` on the row.
- Console: `src/features/dashboards/share-dialog.tsx` (toggle, sample token, iframe snippet); pages under `src/app/embed/` (`dashboard`, `signed`).
- No `frame-ancestors` or `X-Frame-Options` is set anywhere (`next.config.*`, middleware).

## 3. Tasks

One commit per task. Rust first.

### T1 — Claims and verification (`lakehouse-embed`)

Add `iat: Option<f64>` and `jti: Option<String>` to the claims. `verify_embed` takes the maximum lifetime and "now" (inject the clock; no sleeping in tests) and applies section 1. Keep the signature check constant-time and first. Return a small error enum internally for tests and logs; callers map every variant to the one fixed message.

*Accept:* unit tests for each refusal (no `exp`, no `iat`, non-numeric, expired, `iat` in the future, lifetime over the maximum, lifetime exactly at the maximum accepted, 60-second tolerance on both ends, malformed `jti`), and that a wrong signature is refused before any claim is read.

### T2 — The secret

Remove the generate-and-store path. The secret comes from configuration only; unset means signed embedding is unavailable: `POST /api/embed/data` answers 503 with "embedding is not configured"; `embed-info` answers `supported: false` with that reason and no sample token. The sample token gains `iat` (and a `jti`).

*Accept:* tests that nothing reads or writes `console.app_kv` for the secret any more (a mock `ClickHouse` records no such statement); unset secret gives the fixed 503 and `supported: false`; the API config test shows the API starts without it.

### T3 — Withdrawal

Storage where the board already lives, unless that cannot give the one-minute guarantee (then stop and tell the planner): a "revoked before" instant on the board, and a small set of withdrawn `jti` with each token's `exp`. If this needs a new table or column, write the migration or `CREATE … IF NOT EXISTS` in the style the neighbours use, with a why-header; state which store and why in the handoff.

Routes (extend existing ones where a verb already fits; a new route needs its `POLICY_TABLE` entry and its assertion in `tests/route_auth.rs`): withdraw all for a board; withdraw one by presenting the token (the server verifies the signature, reads `jti` and `exp`, and refuses a token with no `jti` with our message; it must not accept a bare `jti` from the caller); read the board's "revoked before" for the Share dialog. `POST /api/embed/data` checks both on every request.

*Accept:* route tests: a token issued before "withdraw all" is refused and one issued after is accepted; a withdrawn `jti` is refused while another token for the same board works; withdrawing needs `dashboard:write`; a token with no `jti` cannot be withdrawn singly; a withdrawn `jti` for board A does not affect board B.

### T4 — Allowed origins and the frame policy

- Store the list on the board; validate on write (section 1); edit through the board write path with `dashboard:write`.
- The embed pages must send the header per dashboard. Find how the two pages under `src/app/embed/` are served and where a per-request response header can be set in this Next version (read `next.config.*`, any `middleware`/`proxy` file, and the Next docs in `node_modules/next/dist/docs` if present; verify, do not guess). The page needs the board's origins for the token it was given: add the smallest read for that. If it needs a backend route, it is `Policy::Public`, takes the same token the embed already presents, returns only the origin list, and for an invalid token returns the same answer as an empty list (no oracle about token validity). Register and assert it.
- Both embed pages get the header: the signed one and the share-token one (`/embed/dashboard/[token]`). A response without a resolvable board gets `frame-ancestors 'none'`.
- The rest of the console is not changed by this task (note in the handoff whether it sends any framing header today).

*Accept:* tests for origin validation (scheme, port, wildcard refused, more than 20 refused, `http://` only for `localhost`); a test of the header value for an empty list, a list, and an unknown token. If the header is set in Next code that the unit tests cannot reach, say exactly how it was checked (for example `curl -I` against the dev server, quoting the header).

### T5 — Console

Share dialog: the allowed-sites editor (add, remove, validation message from the server); "Withdraw all embed tokens" with a confirmation that says existing embeds stop working; "Withdraw one token" (paste, result message); the "not configured" state when `supported: false`; the iframe snippet and the help text say a token needs `iat` and `exp` and how long it may live (read the maximum from `embed-info`, do not hard-code 24). The embed page shows the fixed refusal message for a refused token. Contracts → clients → services → features; no new dependency.

*Accept:* typecheck, lint, unit tests for the new client functions and the dialog's states.

### T6 — Docs

`CHANGELOG.md` `[Unreleased]`: under Security, and the three breaking changes from the feature page stated as breaking with what to do. `docs/OPERATIONS.md`: `EMBED_SECRET` is now the only source, the old `console.app_kv` row, `EMBED_TOKEN_MAX_LIFETIME_SECONDS`, how to sign a token (claims, an example with placeholder values). `.env.example` and `docker-compose.yml` for the new setting (`${X:-}`, no default secret).

## 4. PR slicing

Commits T1…T6 on the phase 0 branch.

## 5. Out of scope (do not build)

Public links' expiry or password (`BI-19`); `BI-26` embed features; secret rotation with overlap; framing headers for the rest of the console; deleting the old `app_kv` row.

## 6. Things the developer must verify, not assume

- Every place a signed token is verified or minted (`embed.rs`, `embed-info`, tests, the console's sample snippet), and that the public share link path shares none of the new checks.
- That "within one minute" holds: name every cache between a withdrawal and the next `embed/data` answer (in the API, in the store's read path such as `FINAL` versus eventual merges, in Next's fetch caching for the embed pages).
- What the embed pages do on a refused token today, so the new message replaces rather than adds.
- That `frame-ancestors` actually reaches the browser for both pages (quote the header from a real response).
- A compose edit is proven by `docker compose --profile '*' config --quiet`; say that `up` was not run.

## 7. Handoff (developer appends one entry per PR)

## 8. Review (planner appends findings per PR)
