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

### SEC-12 partial notes (developer, in progress; nothing committed)

- **T1 written.** `lakehouse-embed/src/lib.rs` (claims `iat`/`jti`, `TokenError`, `verify_embed(token, secret, max, now)`, `unix_now`), `lakehouse-embed/tests/cross_language.rs`; the caller `routes/embed.rs` (fixed message `embed token is invalid or expired`, token without a dashboard now the same message), `routes/dashboard.rs` (sample token gets `iat`/`jti`), `config.rs` (`EMBED_TOKEN_MAX_LIFETIME_SECONDS`, bad value stops start), `tests/query_author_exception.rs` (token now carries `iat`/`exp`). Verified: `cargo test -p lakehouse-embed` 18 unit + 2 cross-language pass; `cargo check -p lakehouse-embed -p lakehouse-api --tests` ok. Not yet run: lakehouse-api tests.
- **T2 written.** `lakehouse-embed/src/lib.rs` (resolver, `console.app_kv` DDL and generator removed), `lakehouse-embed/Cargo.toml` + `rust/Cargo.lock` (dropped now-unused `lakehouse-clickhouse`, `rand`, `tokio`, `wiremock`; nothing added), `lakehouse-api/src/state.rs` (`embed_secret` field removed; callers read `state.config.embed_secret`), `routes/embed.rs` (503 `embedding is not configured`), `routes/dashboard.rs` (`embed-info`: `supported`/`reason`/`maxLifetimeSeconds`, no sample token when unset), `config.rs` and `pipeline_source.rs` (comments), new `tests/embed_tokens.rs`. Verified: `cargo test -p lakehouse-api --test embed_tokens` 4 passed (no `app_kv` statement, no ClickHouse request at all when the secret is unset).
- **T3 written (storage: the board row, no migration).** `lakehouse-bi/src/embed_access.rs` (new: state, `is_withdrawn`, `revoke_all`, `withdraw_token`, `validate_origins`), `lakehouse-bi/src/store.rs` + `lib.rs` (three `ALTER ... ADD COLUMN IF NOT EXISTS` in `ensure_bi_table`: `embed_revoked_before`, `embed_revoked_jti_json`, `embed_origins_json`; carried forward by every `save_board_patch`; `revoke_all_embed_tokens`, `withdraw_embed_token`), `routes/embed.rs` (withdrawal check on every `embed/data`; `POST /api/dashboard/embed-revoke`), `routes/dashboard.rs` (`PUT boards` accepts `embedRevokeAll`/`embedOrigins`; `embed-info` returns `revokedBefore`, `allowedOrigins`, `maxLifetimeSeconds`), `routes/mod.rs`, `policy.rs` (+2 entries, public count 7 to 8), `tests/embed_tokens.rs`, `tests/route_auth.rs`. The origins half of T4 (storage, validation, `POST /api/embed/frame`) landed with it because it shares the same columns and routes.
- **Verified so far:** `cargo test -p lakehouse-api --test embed_tokens --test route_auth`: 21 + 32 passed. Not yet: lib tests, clippy, bi/embed unit tests re-run, sec11_guard, security_regressions.
- **T4 written.** Backend half is in T3's files (see above). Console: `src/lib/embed-frame.ts` (+ `.test.ts`, 13 tests), `src/proxy.ts` (new; Next 16 `proxy` file convention, verified in `node_modules/next/dist/build/templates/middleware.js`). Verified: `bun test src/lib/embed-frame.test.ts` 13 pass; `curl -I` against the dev server :3100 returns `content-security-policy: frame-ancestors 'none'` for `/embed/dashboard/p_nope`, `/embed/signed/a.b.c` and `/embed/other` (the dev API is the old binary, so its `/api/embed/frame` is a 404 and the page fails closed; the non-empty list is NOT verified on a running system).
- **T5 written.** `src/services/contracts/dashboards.ts` (`EmbedInfo`, four methods), `src/services/clients/dashboards.ts` (+ new `dashboards.test.ts`, 8 tests), `src/features/dashboards/embed-access-panel.tsx` (new: allowed-sites editor, withdraw all with confirmation, withdraw one), `share-embed.ts` (+ test, 6), `share-dialog.tsx` (+ new `share-dialog.test.tsx`, 13), `embed-refusal.ts` (+ test, 3), `embed-view.tsx`. Verified: `bun run typecheck` clean, `bun run lint` 0 errors (6 old warnings, none in these files); the new test files pass individually. Full `bun run test` not yet run.
- **T6 written.** `CHANGELOG.md` (Security entry, three breaking changes named), `docs/OPERATIONS.md` (new section, table row), `.env.example`, `docker-compose.yml` (`EMBED_TOKEN_MAX_LIFETIME_SECONDS: ${...:-}`, no default secret), `README.md` (env table). `docker compose --profile '*' config --quiet` ok (`up` NOT run).

### SEC-12 — T1–T6 (developer, 2026-10-10)

Nothing is committed, staged or stashed (instruction: the product owner QAs first). Work tree on `fix/sec-phase-0` over `00aad70`.

**Files per task.** A file touched by two tasks is listed under each; the hunks are separable by the task notes above but not by file, so splitting into commits needs `git add -p`.
- T1: `rust/crates/lakehouse-embed/src/lib.rs`, `tests/cross_language.rs`; callers kept compiling: `lakehouse-api/src/routes/embed.rs`, `routes/dashboard.rs` (sample claims), `config.rs` (`EMBED_TOKEN_MAX_LIFETIME_SECONDS`), `tests/query_author_exception.rs`.
- T2: `lakehouse-embed/src/lib.rs`, `Cargo.toml`, `rust/Cargo.lock`; `lakehouse-api/src/state.rs`, `routes/embed.rs`, `routes/dashboard.rs`, `config.rs`, `pipeline_source.rs` (a comment), `tests/embed_tokens.rs` (new), `tests/upstream_errors.rs`.
- T3 (and the origins half of T4): `lakehouse-bi/src/embed_access.rs` (new), `store.rs`, `lib.rs`; `lakehouse-api/src/routes/embed.rs`, `routes/dashboard.rs`, `routes/mod.rs`, `policy.rs`, `tests/embed_tokens.rs`, `tests/route_auth.rs`; `rust/tests/parity/corpus/embed-data-bad-jwt.json`, `dashboard-embed-info.json`.
- T4 (console): `src/lib/embed-frame.ts`, `embed-frame.test.ts`, `src/proxy.ts`.
- T5: `src/services/contracts/dashboards.ts`, `src/services/clients/dashboards.ts`, `dashboards.test.ts`, `src/features/dashboards/{embed-access-panel.tsx, share-embed.ts, share-embed.test.ts, share-dialog.tsx, share-dialog.test.tsx, embed-refusal.ts, embed-refusal.test.ts, embed-view.tsx}`.
- T6: `CHANGELOG.md`, `docs/OPERATIONS.md`, `README.md`, `.env.example`, `docker-compose.yml`.

**Where withdrawal state is stored, and why.** On the board row in `ClickHouse` `console.bi_board`, three new columns added by the existing `ensure_bi_table` bootstrap (`ADD COLUMN IF NOT EXISTS`, constant defaults): `embed_revoked_before UInt64` ("withdraw all", Unix seconds, tokens with `iat` at or before it are refused), `embed_revoked_jti_json String` (`[{jti, exp}]`, at most 1000, an entry is dropped when `exp + 60` has passed because the verifier accepts a token up to 60 s past `exp`), `embed_origins_json String` (at most 20 sites). No Postgres migration and no new table (the bootstrap is the project's migration mechanism for these tables; the statement count in the existing DDL test went 13 to 16). Why here: it is where the board already lives, `embed_enabled` is read there, and a board delete takes its withdrawals with it. Every save is a full-row `INSERT` (`ReplacingMergeTree`), so `save_board_patch` carries the three columns forward on every write (a test proves a rename does not reset them).

**Every cache between a withdrawal and the next `embed/data` answer: none.** (1) API process: the board is read on every request (`store::get_board`, `FINAL`); there is no cache of boards, tokens or secrets (the old secret cache is gone with the resolver). (2) Store read path: the write is a synchronous `INSERT` (`ChClient::exec`, no `async_insert` anywhere in `lakehouse-clickhouse`), read back with `FINAL` on a single-version key, so there is no eventual-merge delay; the only ordering assumption is `ReplacingMergeTree(created_at)` with a one-second `DateTime` version, the same one every board write already relies on. (3) Next: `embed-view.tsx` fetches with `cache: "no-store"`; `proxy.ts` fetches with `cache: "no-store"`; the `/api` rewrite proxies without caching and `POST` is never cached by a browser. The page's frame header is computed per request in the proxy, so a withdrawn token's page also loses its site list at once. Proved in `a_withdrawal_written_between_two_requests_is_honoured_by_the_second`.

**How the frame header is set.** `src/proxy.ts` (Next 16.1.6 `proxy` file convention, verified in `node_modules/next/dist/build/templates/middleware.js`: `mod.proxy || mod.default`; `matcher: ["/embed/:path*"]`) calls the new public `POST /api/embed/frame` with the page's token (3 s timeout, `RUST_API_URL` read at run time) and sets `Content-Security-Policy` on `NextResponse.next()`. Any failure, non-200, malformed answer or unknown path is `frame-ancestors 'none'`; entries are re-validated against the same origin pattern before they reach a header. Before this work the console sent no `Content-Security-Policy` and no `X-Frame-Options` on any page (checked with `curl -I` before adding the proxy); the proxy matches only `/embed/*`, nothing else changed. Quoted from the running dev server :3100 after the change, `curl -I`:
- `/embed/dashboard/p_nope` -> `HTTP/1.1 200 OK`, `content-security-policy: frame-ancestors 'none'`
- `/embed/signed/a.b.c` -> `HTTP/1.1 200 OK`, `content-security-policy: frame-ancestors 'none'`
- `/embed/other` -> `HTTP/1.1 404 Not Found`, `content-security-policy: frame-ancestors 'none'`
The dev API is the old binary (its `/api/embed/frame` answers 500, "unclassified route"), so these show the fail-closed path for both embed pages. The non-empty list (`frame-ancestors https://...`) is **not verified on a running system**; it is verified by `embed-frame.test.ts` (the header string) and `tests/embed_tokens.rs` (the API's answer). Needs a restarted API.

**Every place a signed token is minted or verified.** Minted: `lakehouse_embed::sign_embed` called from `routes/dashboard.rs` `embed_info_body` (the sample), from tests (`tests/embed_tokens.rs`, `tests/query_author_exception.rs`, `lakehouse-embed` unit tests), and, outside the repo's code, by the customer's server (the Share dialog's snippet, now `share-embed.ts` `signSnippet`). The TypeScript `embed-jwt.ts` no longer exists. Verified: `verify_embed` called from `routes/embed.rs` in three places: `data` (the answer), `frame_origins` (the framing list), `revoke_token` (withdraw one); `cross_language.rs`. The public share link (`public_dashboard`, `get_board_by_token`) calls none of them and none of the new checks; `the_public_link_ignores_every_new_check` proves it with everything withdrawn. Its iframe page `/embed/dashboard/<token>` does need listed sites (decision 5).

**Deviations and decisions the plan did not spell out** (each one is small; tell me if you want any reversed):
1. A token with no dashboard, or an empty one, is now the same `401` "embed token is invalid or expired" (it was `400 no_resource`, and an empty id fell to `403 embedding_disabled`): it is a token refusal, and the plan wants one message.
2. A token whose `exp` is before its `iat` is refused (`TokenError::Lifetime`); the plan's `exp - iat <= max` would otherwise accept it.
3. A withdrawn `jti` is kept until `exp + 60 s`, not `exp`, because the verifier accepts a token that long past `exp` (section 1 said "until the token's own `exp` has passed").
4. No new route for "withdraw all" or "edit sites": `PUT /api/dashboard/boards` takes `embedRevokeAll: true` (false is a 400) and `embedOrigins: [..]` (same `dashboard:write`). New routes: `POST /api/dashboard/embed-revoke` (`dashboard:write`) and `POST /api/embed/frame` (`Public`, so the public-entry count in `policy.rs` went 7 to 8; both are in `POLICY_TABLE` and walked by the existing loops in `tests/route_auth.rs`, plus one explicit test).
5. `embed-revoke` answers `400` (not `401`) for a token that does not verify, so the console's `apiFetch` does not treat it as a lost session. It refuses a token for another dashboard with "that token is for a different dashboard."
6. `lakehouse-embed` dropped its now-unused dependencies `lakehouse-clickhouse`, `rand`, `tokio` and the dev-dependency `wiremock` (lockfile: 4 lines removed, nothing added).
7. `lakehouse-bi` gets `Board.embed_access` with `#[serde(skip)]`, so the board listing, the parity corpus shapes and the export never carry the withdrawn ids.
8. A corrupt `embed_revoked_jti_json` fails closed (every token of that board is refused until the board is saved again); a corrupt origins column allows no site.
9. Console: the Share dialog now calls `dashboardService` for embed state (it called `apiFetch` itself); the public-link/embed toggles still use the dialog's own `putBoard`, unchanged.

**Existing tests changed, and why** (each pinned old behaviour on purpose or by count): `lakehouse-embed` `accepts_no_expiry` (pinned "a token without exp never expires") replaced by `rejects_a_token_with_no_exp`; `accepts_far_future_expiry` replaced by `a_ten_year_token_is_refused`; the three `get_embed_secret_*` tests removed with the resolver; `tests/cross_language.rs` (`ts_generated_token_verifies_in_rust` asserted `exp == None` was accepted; now asserts the signature verifies and the claims are refused with `MissingExp`); `lakehouse-bi` `ensure_bi_table_runs_ddl_at_most_once_per_process` 13 to 16 statements; `policy.rs` `exactly_seven_public_entries_exist` renamed and set to 8; `tests/query_author_exception.rs` token now carries `iat`/`exp`; `tests/upstream_errors.rs` `the_embed_data_route_never_carries_the_database_text` also accepts `503` (with no secret the route answers before any query); `rust/tests/parity/corpus/embed-data-bad-jwt.json` and `dashboard-embed-info.json` edited by hand to the new body/shape (live replay, **not replayed**).

**Commands run (foreground, one at a time) and counts.**
- `cd rust && cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-embed -p lakehouse-bi -p lakehouse-api --all-targets --all-features --locked -- -D warnings`: clean (after `touch` of the three `src/lib.rs`).
- `cargo test -p lakehouse-embed -p lakehouse-bi`: embed 15 unit + 2 cross-language; bi 94 unit + 1 `specs_match_typescript`; all pass.
- `cargo test -p lakehouse-api --lib`: 1523 passed, 0 failed, 1 ignored (already ignored).
- `cargo test -p lakehouse-api --test sec11_guard --test route_auth --test security_regressions --test embed_tokens --test query_author_exception --test upstream_errors`: 4, 32, 10, 21, 6, 6 passed (`upstream_errors` re-run alone after the edit above). `sec11_guard` still passes, with no allowlist entry added.
- `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings, none in files of this work; `bun run test` 980 pass, 1 skip, 0 fail (981 tests, 108 files).
- `docker compose --profile '*' config --quiet`: ok. `up` was NOT run.
- Test executables over 20 MB deleted from the shared target after each broad run.

**Not verified.** No `--workspace` run and no other `lakehouse-api` integration target (only those listed). `tests/parity.rs` (live replay) not run. A non-empty `frame-ancestors` list on a running system, and a browser actually blanking a frame, not seen. `docker compose up` not run. The 60-second effect time is shown by the no-cache argument and the between-requests test, not by a stopwatch.

**Known limits (for the review).** (a) "Withdraw all" compares the API's clock with the token's `iat`: a token minted by a clock up to 60 s ahead of ours has an `iat` later than the withdrawal and survives it until it expires (bounded by the lifetime limit); documented in `docs/OPERATIONS.md` and the changelog. (b) Full-row `INSERT` saves read the board first and write it back; a withdrawal racing an unrelated save of the same board within milliseconds could be overwritten by the save that read the older row. The store functions re-read just before writing, which makes the window small, not zero; a separate append-only table would remove it and was not built because the plan said to store on the board. (c) `frame-ancestors` alone: browsers that ignore CSP3 `frame-ancestors` are not covered (no `X-Frame-Options` is sent, as it cannot express a list). (d) Not covered by this work: an existing `embed_secret` row in `console.app_kv` is not deleted (as decided).

### SEC-12 — review round 1 (developer, 2026-10-10)

Frontend only; no Rust touched; nothing committed.

- **Finding 1 (copy).** No helper with a fallback existed; about 45 call sites call `navigator.clipboard.writeText` directly. Added `src/lib/copy-text.ts` (`copyText(text): Promise<boolean>`: async API when present and the context is secure, else a temporary off-screen textarea plus `execCommand("copy")`, restoring focus; `false` when neither works) with `copy-text.test.ts` (5). Used by every copy button in `share-dialog.tsx`; "Copied" shows only on success, a failure shows "Could not copy — select the text and copy it". **Follow-up, not changed:** direct `navigator.clipboard` calls remain in `features/overview/activity-columns.tsx`, `queries/saved-query-columns.tsx`, `governance/{audit,maintenance,data-quality,lineage,classification,policy,ingestion}-columns.tsx`, `catalog/{sample-inspector,data-explorer-actions,asset-overview,catalog-namespace-columns}.tsx`, `copilot/{chat-messages,copy-button}.tsx`, `pipelines/{run-log-console,run-inspector,pipeline-columns}.tsx`, `components/copyable.tsx`, `admin/{service-identities-page,service-identities-columns,user-columns,tenants-columns,roles-columns}.tsx`, `agents/{approval,employee,run}-columns.tsx`.
- **Finding 2 (sample token).** A labelled read-only monospace field "Sample token (valid 1 hour)" (truncated by CSS), with **Copy token** and **Preview** beside it.
- **Finding 3 (less text).** Structure as asked: public link one line; signed embedding one line, then one sentence ("Tokens need iat and exp and may live at most <max>."), the sample token row, collapsed `<details>` "How to sign a token" (EMBED_SECRET sentence, code, Copy code) and "Withdraw tokens" (confirmation text kept; "last withdrawn <relative>" beside the title). Unsupported: toggle disabled and one line. Sites: one-line description, chips, input + Add, one-line help, tooltip carries the browser/localhost note, empty-list line, Save sites only when changed, "Saved" for 2.5 s. `src/lib/embed-origins.ts` (+ test, 7) validates on Add with the server's rule (https, http only for localhost, no wildcard/path, at most 20, no duplicates); the server message is still shown if it refuses. Native `<details>` (no accordion primitive exists in `components/ui`). Dialog keeps `max-h-[85vh]` + scroll. Withdraw tools now show only while signed embedding is on (they showed whenever supported before). Controls and service calls unchanged.
- **Files:** `src/lib/{copy-text,embed-origins}.ts` + tests (new); `src/features/dashboards/{share-dialog.tsx, embed-access-panel.tsx, share-embed.ts}`, `share-dialog.test.tsx` (now 19), `share-embed.test.ts`. Existing test changes: the wildcard-from-server test became "invalid site never becomes a chip" plus a server-refusal test; the Add test expects the normalised lower-case site; "Saved." is "Saved".
- **Commands:** `bun run typecheck` clean; `bun run lint` 0 errors, 6 old warnings; `bun run test` 1001 pass, 1 skip, 0 fail (1002 tests, 110 files).
- **Not verified:** no browser. The look at 360 px, the dark theme and the real insecure-origin copy are untested by eye; `execCommand` is exercised under happy-dom only.

## 8. Review (planner appends findings per PR)

### SEC-12 — T1–T6 and review round 1 (reviewer, 2026-10-10)

- **Backend: no finding.** Probed on a rebuilt API with tokens signed
  locally with the dev secret: a valid one-hour token rendered six tiles; a
  token with no `exp`, no `iat`, a 25-hour lifetime, already expired, issued
  in the future, or signed with another secret answered 401 "embed token is
  invalid or expired"; withdrawing one token by presenting it refused that
  token and left a sibling working; a token without `jti` could not be
  withdrawn singly and said why; after "withdraw all" the older token was
  refused and one signed afterwards worked; the embed page answered
  `frame-ancestors 'none'` with no site listed and for an unknown token, and
  `frame-ancestors https://app.customer.example` once that site was listed;
  an `http://` site and a wildcard were refused; withdrawing without a
  session was 401.
- **Round 1, from the owner's QA of the Share dialog (fixed).** Copy
  buttons did nothing on an insecure origin (`navigator.clipboard` is
  undefined there); the sample token was only reachable inside the preview
  link; the dialog carried far more text than controls. One copy helper with
  a fallback, a visible sample-token row, and the dialog restructured so
  explanation is one line or behind a disclosure. The owner re-ran it on
  2026-10-10: copy works on their origin, a pasted sample token was
  withdrawn, the amount of text is right.
- **Accepted deviations.** A token with no dashboard in it gets the same
  401 as any other refusal; a withdrawn `jti` is kept until 60 seconds past
  its `exp`, matching the verifier's tolerance; `embed-revoke` answers 400,
  not 401, for a token that does not verify, so the console does not sign
  the user out; `lakehouse-embed` dropped four unused dependencies; one new
  public route, `POST /api/embed/frame`, which answers an empty list for
  any invalid token.
- **No open BLOCKER.**

Verified by the reviewer on the final tree: `bun run typecheck` clean,
`bun run lint` 0 errors, `bun run test` 1001 pass, 1 skip (on `main`);
`cargo fmt --check` clean; `cargo clippy -p lakehouse-embed -p lakehouse-bi
-p lakehouse-api --all-targets --all-features --locked -- -D warnings`
clean; `lakehouse-embed` 15 + 2, `lakehouse-api --lib` 1523 (1 ignored),
`embed_tokens` 21, `route_auth` 32, `sec11_guard` 4 passed;
`docker compose --profile '*' config --quiet` exit 0. Rust was not changed
after those runs.

*Not verified:* a browser actually blanking a frame from an unlisted site
(the header is correct; the effect was not observed); the API started with
`EMBED_SECRET` unset on a running system (tests only); the dialog in the
light theme and at phone width; `cargo test --workspace`; the parity
corpus (two files hand-edited, not replayed).

Carried forward: a withdrawal racing another save of the same board within
milliseconds could be overwritten, because every save writes the whole
row; only `frame-ancestors` is sent, so a browser that ignores it is not
covered; about 45 other copy buttons in the console call the clipboard
directly and fail the same way on an insecure origin. Each needs a backlog
item.

