# Login throttling and session cleanup — Implementation Plan

**Status:** built, in review — review 2 (2026-10-03) has two open BLOCKERs (3, 5), see §8.
Written 2026-10-02 by the planner (Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page (requirements and acceptance checklist):**
`docs/core/features/login-protection-and-session-cleanup.md`. Backlog
`SEC-2`, `SEC-5`.

**Base:** `main` at or after `0277ebd`. Branch
`feat/login-throttle-session-cleanup`.

**Why.** `README.md` and `SECURITY.md` list "no login rate limiting beyond
logging" and "sessions and service tokens have no rotation/cleanup job" as
known limitations. Both are found by any customer security review.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| What is throttled | Password login only (`POST /api/auth/login`). Not single sign-on, not `change-password`. |
| Key | Per account: a SHA-256 of the submitted email, trimmed and lower-cased. Computed the same way whether or not the account exists. |
| Not keyed by IP | The API is reached through the console's server; the client address is not trustworthy without proxy-header handling, which is out of scope. |
| Defaults | 5 failures within 900 s lock the key for 300 s. Each is an env setting. |
| No off switch | An unset, zero, negative or unparseable value falls back to the default. The throttle cannot be disabled by configuration. |
| Storage | Postgres, so it survives a restart and is shared if the API ever runs on more than one node. |
| Locked response | `429` with a `Retry-After` header and a fixed message. Identical for an existing and a non-existing account. |
| Cleanup runs where | Inside `lakehouse-api`, as a background task. Not a Dagster job: the default stack runs without Dagster. |
| Retention | 30 days by default, an env setting. |
| What cleanup deletes | Sessions expired or revoked longer ago than the retention; revoked service credentials older than the retention; throttle rows that are no longer locked and whose window has passed. Never an active service credential, never a live session. |

The defaults are the planner's proposal; the product owner has not signed
them. They are settings, so changing them later is not a code change.

## 2. What exists today (anchors, verified at `0277ebd`)

Re-find by symbol if a line number has moved.

- **Login:** `rust/crates/lakehouse-api/src/routes/auth.rs`, `login`
  (~116). It calls `auth.local.authenticate(&credential)` and logs
  `tracing::warn!("login failed")` on failure. The module doc comment
  (~11–26) says a rate limiter is "future work, deliberately not built
  here" — that paragraph must be rewritten by this change.
- **Password verification:** `rust/crates/lakehouse-auth/src/password.rs`
  (`LocalPasswordAuthenticator` ~207). It pays the same Argon2id cost for
  an unknown email and a wrong password. Keep that property.
- **Sessions:** `rust/crates/lakehouse-auth/src/session.rs`
  (`create_session`, `validate_session`, `revoke_session`,
  `DEFAULT_SESSION_TTL` = 24 h).
- **Service credentials:** `rust/crates/lakehouse-auth/src/service_token.rs`.
- **Tables:** `rust/migrations/0019_auth.sql` — `session` (`expires_at`,
  `revoked_at`), `service_credential` (`revoked_at`; no expiry column).
- **Sessions page query:** `rust/crates/lakehouse-store/src/sessions.rs`
  (~98–101) lists only `revoked_at IS NULL AND expires_at > now()`, so
  deleting dead rows does not change what it shows.
- **Errors:** `rust/crates/lakehouse-core/src/error.rs` — `ApiError` has no
  `429` variant. `rust/crates/lakehouse-api/src/error.rs`
  (`impl IntoResponse for ApiRejection`, ~39) builds the response from
  `self.0.status()`.
- **Audit:** `lakehouse_store::audit::{insert, NewAuditEvent}`; see
  `oidc_login_audit_event` and `session_revoke_audit_event` at the bottom of
  `routes/auth.rs` for the shape.
- **Background tasks:** `main.rs` spawns none today. `routes/ai/mod.rs`
  (~843) is the only non-test `tokio::spawn`.
- **Config:** `rust/crates/lakehouse-api/src/config.rs`; `or_default` is the
  env helper; see `gold_export_max_rows` for a numeric setting with a
  documented fallback.
- **Console:** `src/features/auth/login-page.tsx`,
  `src/services/clients/auth.ts` (`login` ~57, via `parse`).
- **Latest migration:** `0054_gold_publication.sql`. Next free is `0055` —
  check every open branch on `origin` before taking it.

## 3. Tasks

One commit per task, in order. Rust is T1–T4; do them in one sitting so the
migration lands with the code that uses it. T5 is TypeScript, T6 is docs:
neither runs cargo.

### T1 — A `429` error

- Add `ApiError::TooManyRequests { message: String, retry_after_secs: u64 }`
  to `lakehouse-core`, status `429`.
- `ApiRejection`'s `IntoResponse` sets a `Retry-After: <secs>` header for
  that variant. No other variant's response changes.
- Every `match` over `ApiError` in the workspace must still be exhaustive;
  do not add a wildcard arm to get there.
- **Accept:** unit tests beside the existing status tests in
  `lakehouse-api/src/error.rs`: status is `429`, the header is present and
  equals the seconds given, the body carries the fixed message.

### T2 — Throttle storage

- Migration `0055_login_throttle.sql`, with a why-header:
  `login_throttle (key_hash TEXT PRIMARY KEY, failures INT NOT NULL CHECK
  (failures >= 0), window_started_at TIMESTAMPTZ NOT NULL, locked_until
  TIMESTAMPTZ)`. The header must say the key is a hash and why (no email is
  stored for accounts that do not exist).
- New module `rust/crates/lakehouse-auth/src/throttle.rs`, exported from
  `lib.rs`:
  - `ThrottlePolicy { max_failures: u32, window: Duration, lockout: Duration }`.
  - `key_for(email: &str) -> String` — SHA-256 hex of the trimmed,
    lower-cased input. Reuse `crate::token::hash_token` if it fits;
    otherwise say in a comment why not (rule 4).
  - `locked_until(pool, key) -> Result<Option<OffsetDateTime>, AuthError>` —
    `Some` only while `locked_until > now()`.
  - `record_failure(pool, key, policy) -> Result<Option<OffsetDateTime>, AuthError>` —
    **one** SQL statement (`INSERT … ON CONFLICT DO UPDATE`): start a new
    window if the old one has passed, increment otherwise, and set
    `locked_until = now() + lockout` when the count reaches `max_failures`.
    Returns the lock time if this call set one. It must be atomic: two
    concurrent failures must both be counted.
    *Corrected in review 1 (SHOULD-FIX 1):* a failure on a key whose lock
    has expired also starts a new window at 1, exactly like a passed
    window, and clears the expired `locked_until` (review 2, BLOCKER 5).
  - `clear(pool, key) -> Result<(), AuthError>`.
  - All values bound. `# Errors` on every `Result` fn.
- **Accept:** `sqlx::test`s: below the limit no lock; the Nth failure
  locks; a failure after the window has passed starts at 1; `clear` removes
  the row; an expired lock reads as not locked; ten concurrent
  `record_failure` calls end with `failures = 10` (or locked), not fewer.

### T3 — Throttle the login route

- `Config`: `login_max_failures` (`LOGIN_MAX_FAILURES`, 5),
  `login_failure_window_secs` (`LOGIN_FAILURE_WINDOW_SECS`, 900),
  `login_lockout_secs` (`LOGIN_LOCKOUT_SECS`, 300). Zero, negative or
  unparseable falls back to the default; say so in each doc comment. Add
  them to the `Debug` impl like the neighbouring fields.
- `login`, in this order:
  1. Parse the body. Compute `key_for(&email)`.
  2. If `locked_until(key)` is `Some(t)`: return
     `TooManyRequests` with a fixed message ("Too many failed sign-in
     attempts. Try again later.") and `retry_after_secs` = seconds until
     `t`, rounded up, at least 1. **Do not** call `authenticate` — a locked
     key must not cost an Argon2 verification.
  3. Otherwise authenticate. On failure: `record_failure`; if that call
     set a lock, write one audit event (below); return the same `401` as
     today. On success: `clear(key)`, then continue as today.
- The `401` body and status for a wrong password must be byte-identical to
  today's. The `429` must be identical for an existing and a non-existing
  email.
- Audit event when a lock is set: `action = "auth.login_locked"`,
  `resource_kind = "login_key"`, `resource_id` = the first 12 hex characters
  of the key hash, `outcome = "refused"`, no principal, **no email, no
  password, no full hash**. Best-effort: a failed audit write is logged,
  not propagated (the posture `record_publication_audit` in
  `routes/gold.rs` uses).
- A storage error from the throttle is returned as the classified error it
  is. Do not fall through to an unthrottled login.
- Rewrite the module doc comment's "Non-enumeration on login" section to
  describe what now exists. Remove the "future work" sentence.
- **Accept:** integration tests in
  `rust/crates/lakehouse-api/tests/` (new file `login_throttle.rs`, using
  the shared harness with env overrides for small limits):
  - N wrong passwords → `401` each; the next attempt with the **correct**
    password → `429` with `Retry-After`.
  - The same sequence for an email that does not exist returns the same
    statuses and the same bodies.
  - A correct login before the limit resets the count.
  - One `auth.login_locked` row exists after a lock, and its JSON contains
    neither the email nor the password.
  - With the lock expired (set a 1-second lockout in the test), the correct
    password works.

### T4 — Cleanup

- `Config`: `auth_retention_days` (`AUTH_RETENTION_DAYS`, 30; same
  fallback rule).
- New module `rust/crates/lakehouse-auth/src/cleanup.rs`:
  `purge(pool, retention: Duration, throttle_window: Duration) ->
  Result<PurgeCounts, AuthError>` with three `DELETE`s:
  - `session` where `expires_at < now() - retention` or
    `revoked_at < now() - retention`;
  - `service_credential` where `revoked_at < now() - retention`;
  - `login_throttle` where `(locked_until IS NULL OR locked_until < now())`
    and `window_started_at < now() - throttle_window`.
  `PurgeCounts` carries the three row counts.
- `main.rs`: after the bootstraps, if a Postgres pool exists, spawn one
  task that calls `purge` once at start and then every hour
  (`tokio::time::interval`, missed ticks skipped). It logs the counts at
  `info` and an error at `warn`; an error never ends the loop and never
  takes the process down. With no pool it logs once that cleanup is not
  running and spawns nothing.
- **Accept:** `sqlx::test`s seeding one row of each kind on each side of
  the cut-off: a live session, a session expired yesterday, a session
  expired past the retention, a revoked credential past the retention, an
  active credential, a throttle row inside its window, one past it, one
  still locked. Assert exactly which remain. One test that `purge` on empty
  tables returns zeros.

### T5 — Console

- `src/services/clients/auth.ts`: confirm `login` surfaces the server's
  message for a `429`. If `parse` drops it, fix that there, not in the
  page.
- `src/features/auth/login-page.tsx`: on a `429`, show the server's message
  and, when a `Retry-After` value is available, the wait in whole minutes
  ("Try again in about 5 minutes."). Do not show a countdown.
- **Accept:** a component test: a `429` response shows the wait message;
  a `401` still shows the existing credentials message.

### T6 — Documentation

- `README.md`: remove the two limitation bullets this closes ("No login
  rate limiting beyond logging", "Sessions and service tokens have no
  rotation/cleanup job"); replace the second with what remains true —
  active service tokens do not expire. Add the four env settings to the
  configuration table.
- `SECURITY.md`: update "Scope Notes" the same way.
- `.env.example`: the four settings, commented, with the defaults.
- `docs/OPERATIONS.md`: a short section — what locks, for how long, how an
  operator clears a lock by hand
  (`DELETE FROM login_throttle WHERE key_hash = …`, and how to compute the
  hash), that the first admin can be locked, and what the cleanup deletes
  and when.
- `CHANGELOG.md`: two or three plain sentences a customer can read first,
  then the technical detail.
- Do not edit anything under `docs/core/`.

## 4. PR slicing

One pull request, T1–T6. The change is one concern, and the limitation
bullets should disappear in the same merge that makes them untrue.

## 5. Out of scope (do not build)

- Throttling by IP; trusting `X-Forwarded-For`.
- An admin "unlock" action in the console.
- Throttling `change-password`, OIDC, or service-token authentication.
- Expiry or rotation of active service credentials.
- CAPTCHA, two-factor sign-in, password-strength rules.

## 6. Things the developer must verify, not assume

- That `0055` is free on every open branch on `origin`. Other people are
  working in this repository; if it is taken, take the next free number and
  say so.
- Whether login looks an account up case-sensitively. The throttle key is
  lower-cased regardless; report what the lookup does, because a mismatch
  means two spellings of one email share a throttle key but not an account.
- That `Retry-After` survives the console's proxy to the browser. If the
  console's fetch layer drops the header, say so; T5 then shows the message
  without the minutes.
- That adding the `ApiError` variant does not change any existing response.
- That the hourly task does not keep the process from shutting down
  cleanly.

## 7. Handoff (developer appends)

### `feat/login-throttle-session-cleanup` (developer, 2026-10-03)

Base: `origin/main` at `0277ebd`. One commit per task, in plan order:

- `daa562a` — T1, `ApiError::TooManyRequests` + `Retry-After` rendering.
- `df4b8f1` — T2, migration `0055`, `throttle.rs`, integration tests.
- `98567a1` — T3, config fields, `ThrottlePolicy` in `AppState`, throttled
  `login`. Carries the `auth_retention_days` field T4 consumes (one
  struct-block edit; splitting it would have left a commit whose
  `Debug`/`from_map` did not compile).
- `260874f` — T4, `cleanup.rs` + hourly `spawn_auth_cleanup`.
- `f1128ca` — T5, console: `too_many_requests` code, `Retry-After` read,
  login-page wait message, notify hint for the new code.
- `71d3198` — T6, README/SECURITY limitation rewrites, Configuration
  table, `.env.example`, OPERATIONS.md section, CHANGELOG.
- `130f4b6` — T3's acceptance tests (`tests/login_throttle.rs`), written
  after the T6 docs commit when the handoff draft found them missing —
  five tests, one per acceptance bullet in the plan's T3.

Deviations from the plan, none of scope:

- `ThrottlePolicy` gained `#[derive(Debug, Clone)]` (the concurrent test
  moves a clone into each spawned task; the struct has no `Copy`).
- `PurgeCounts` field names are `sessions`/`service_credentials`/
  `throttle_entries` — clippy's `struct_field_names` rejects the uniform
  `_rows` postfix; renaming beat an `#[allow]`.
- `login_failure_window_secs`/`login_lockout_secs`/`auth_retention_days`
  are `u32` (not `u64`): the `u64` version forced `as i64` casts
  (`cast_possible_wrap`); `u32` converts losslessly via `i64::from`.

§6 verifications:

- `0055` free: enumerated every remote branch (`git ls-remote --heads`)
  and grepped each tree's `rust/migrations/` — no `0055` anywhere.
- Case sensitivity: account lookup is `WHERE u.email = $1`
  (`password.rs::verify`) — case-sensitive; the throttle key lower-cases.
  So `Alice@x.com` and `alice@x.com` are different accounts sharing one
  throttle key: an attacker hammering either spelling locks both. That is
  the safe direction; no change made.
- `Retry-After` through the proxy: `next.config.ts` rewrites `/api/*` as
  a same-origin pass-through proxy (no header filtering in Next.js
  rewrites), and the client reads it from that same-origin response.
  Verified by code read, not by a live 429 round trip — *not exercised
  end to end* (below).
- No existing response changed: the new enum variant only adds arms;
  `lakehouse-core`'s 46 status/rendering tests pass.
- Shutdown: the cleanup task is a bare `tokio::spawn` never joined by
  `with_graceful_shutdown`; main returning drops it. A purge caught
  mid-run at shutdown leaves its finished `DELETE`s applied and the rest
  to the next hourly run — the deletes are idempotent.

Verification actually run, on the final commit `71d3198`, foreground:

- `cargo fmt --check` (rust/) — pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  (rust/) — pass, zero warnings.
- `cargo test -p lakehouse-core` — **46 passed, 0 failed**.
- `cargo test --workspace --no-run -j 1` — every test binary compiles,
  0 errors (the `-j 1` matters: parallel linking OOM-kills rustc on this
  7 GB machine).
- `bun run typecheck` — pass. `bun run lint` — 0 errors, 6 warnings, all
  pre-existing in files this PR does not touch (`data-table.tsx`,
  `sidebar.tsx`, `alerts-page.tsx`, `open-format-card.test.tsx`,
  `use-data-table.ts`, `dashboard-specs.ts`).
- `bun run test` — **301 pass, 0 fail** (56 files).

*Not verified, with reason:* every DB-backed test (`#[sqlx::test]` in
`lakehouse-auth`/`lakehouse-api`, including `tests/throttle.rs` and
`tests/login_throttle.rs`) — this environment has no reachable Docker
daemon (docker.sock permission denied; no passwordless sudo), and
`lakehouse-test-support` panics without one by design. The acceptance
tests compile clean (`cargo test --workspace --no-run -j 1`, 0 errors)
but have never executed. CI runs `cargo test --workspace --locked` on
the PR with Docker; that run is the gate for these.

Python/compose lines of the verification block: not run — this PR
touches no Python and no compose file.

### `feat/login-throttle-session-cleanup` (developer, 2026-10-03, review fix round)

Fixes every BLOCKER and SHOULD-FIX from the planner's review of the
first handoff, as commits `656d68c` (code) and `2de51bf` (docs) on the
same branch.

- **Blocker 1** (`rust/crates/lakehouse-api/src/routes/auth.rs`): the
  login handler now records a failure only for
  `AuthError::InvalidCredentials`; every other `AuthError` propagates
  through `From<AuthError> for ApiError` (classified 500) and counts
  nothing. New end-to-end test
  `a_storage_failure_during_login_is_not_rendered_or_counted_as_a_wrong_password`
  renames `auth_identity` mid-login: asserts 500 with the classified
  message for both a wrong and a correct password, zero
  `login_throttle` rows, then first-try success after the table is
  restored.
- **Blocker 2** (`rust/crates/lakehouse-auth/tests/throttle.rs`): added
  the missing `use lakehouse_test_support as _;` — the whole file had
  been compile-verified only; the reviewer has since run it green.
- **Blocker 3** (`rust/crates/lakehouse-auth/src/cleanup.rs`): the test
  module is rewritten as an exact-survivor matrix (`token_hash`, not
  the non-existent `session_hash`; `#[sqlx::test(migrations =
  "../../migrations")]`; FK-valid fixtures through seeded `app_user` /
  `service_identity`), covering live/20-hour-expired/3-day-expired
  sessions, revoked vs active credentials, and
  in-window/past-window/still-locked throttle rows, asserting exact
  surviving sets rather than counts.
- **SHOULD-FIX 1**: `record_failure`'s upsert resets to `failures = 1`
  with a fresh window once the window has lapsed or the lock has
  expired; new test
  `a_failure_after_the_lock_expires_starts_the_count_fresh`.
- **SHOULD-FIX 2**: the concurrency test now asserts `failures == 10`
  and `locked.is_some()` exactly.
- **SHOULD-FIX 3**: new
  `src/features/auth/login-page.test.tsx` (2 tests): 429 renders the
  server message plus "Try again in about 5 minute(s)." for
  `Retry-After: 300`; 401 renders exactly "Invalid email or password."
  and never leaks `invalid_or_expired`.
- **Blocker 4 / SHOULD-FIX 4** (`README.md`, `docs/OPERATIONS.md`,
  `CHANGELOG.md`, `.env.example`): no document claims a
  `LOGIN_MAX_FAILURES=0` off switch anymore; all four now state that
  0/negative/unparseable fall back to the default. OPERATIONS.md no
  longer claims throttle rows purge after `AUTH_RETENTION_DAYS` (now:
  unlocked rows with a lapsed window, no retention gate — same fix in
  `cleanup.rs`'s `purge` doc comment) and gains manual-unlock SQL (by
  `key_hash`, mirroring `throttle::key_for`; tables unqualified in
  `public` — verified no `search_path` override exists), the
  first-admin-lockout caveat, and the active-service-tokens-never-
  expire note.
- **SHOULD-FIX 5**: `PurgeCounts` and `ThrottlePolicy` fields
  documented; both `allow(missing_docs, reason = "self-documenting")`
  exceptions removed.

Verification, run in the foreground on `2de51bf`:

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` — clean (first run failed on an unused variable in the new
  acceptance test; fixed by asserting the classified message also for
  the correct-password attempt, then re-run clean).
- `cargo test -p lakehouse-core` — 46 passed, 0 failed.
- `cargo test --workspace --no-run -j 1` — 0 errors (all test
  binaries compile).
- `bun run typecheck` — clean. `bun run lint` — 0 errors, 6 warnings
  (unchanged, pre-existing, untouched files). `bun run test` — 303
  passed, 0 failed (301 before; +2 new login-page tests).
- DB-backed suites (`throttle.rs`, `cleanup.rs`, `login_throttle.rs`,
  route auth tests): **not run** — still no Docker daemon on this
  machine (docker.sock permission denied, no passwordless sudo). Same
  caveat as the first handoff; CI runs them on the PR. Everything else
  in this round is compile- or execution-verified as quoted above.
- Python and compose lines: not run — no Python or compose file
  touched.

### `feat/login-throttle-session-cleanup` (developer, 2026-10-03, review 2 fix round — NOT a handoff)

This is a stop, not a green handoff: the DB-backed suites were not run
on this machine and this branch adds code they must verify. Per review
2's instruction ("If Docker is not available, say so and stop; the
reviewer runs them"), the reviewer runs them before merge.

Fixes (reviewer's branch `claude/gracious-albattani-vw5wsb` merged in
first, fast-forward to `38fce06`; one commit per finding as required):

- **Blocker 5** (`52bb2e1`, `rust/crates/lakehouse-auth/src/throttle.rs`):
  the `locked_until` CASE in `record_failure` now clears the lock to
  `NULL` under the same reset predicate, before the ELSE that preserved
  the stale expired lock. Exactly the reviewer's experiment. The
  expired-lock clause fires once; a key can always lock again after its
  window fills. Module doc updated with the why and the review
  citation.
- **Blocker 3** (`805641e`, `rust/crates/lakehouse-auth/src/cleanup.rs`):
  `use lakehouse_test_support as _;` added to the inline test module,
  with the why-comment. Exactly the reviewer's experiment.
- **SHOULD-FIX 6 + nit** (`d37b9fa`): "5 minute(s)" → proper
  pluralisation in `login-page.tsx` with the test asserting it;
  README's missing known-limitations bullet (active service tokens
  never expire); OPERATIONS.md drops the untrue "(with a logged
  warning)" and cites `MissedTickBehavior::Skip` instead; the
  "mirrors exactly" unlock-SQL nit softened to "matches ... for an
  ordinary address" with the trim/lower caveats spelled out.

Verification actually run, in the foreground, on `d37b9fa`:

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` — clean.
- `cargo test -p lakehouse-core` — 46 passed, 0 failed.
- `cargo test --workspace --no-run -j 1` — 0 errors.
- `bun run typecheck` — clean. `bun run lint` — 0 errors, 6
  pre-existing warnings (untouched files). `bun run test` — 303
  passed, 0 failed (the pluralisation test asserts the new wording).
- `cargo test -p lakehouse-auth` — **attempted, not runnable here**:
  the harness `ctor` aborts every test binary at process start
  (`lakehouse-test-support: failed to start the Postgres testcontainer
  (is Docker running and reachable?)` → SIGABRT; docker.sock is
  root:docker, this user is not in the docker group, no sudo, no local
  Postgres). No pass counts are producible on this machine. The
  reviewer's experiments (8/8 throttle with the blocker 5 line; 2/2
  cleanup with the blocker 3 line) are the only executed verification
  of these two fixes; the full suites
  (`cargo test -p lakehouse-auth`, `cargo test -p lakehouse-api
  --test login_throttle`) must run on the fixed branch before merge.

Python/compose lines: not run — no Python or compose file touched.

## 8. Review (planner appends)

### Review 1 — planner, 2026-10-03, on `5754a1e`

Reality check first: `feat/login-throttle-session-cleanup` is on `origin`
at `5754a1e`, nine commits over `0277ebd`, hashes match the handoff. No PR
yet (correct — the planner opens it).

#### BLOCKER 1 — every authentication error is now a "wrong password"

`routes/auth.rs`, `login`: `if let Ok(principal) = auth.local.authenticate(..)
… else { record_failure; 401 }`. The `else` catches **every** `AuthError`,
not only `InvalidCredentials`. Before this change `?` turned
`AuthError::Database` / `AuthError::Hash` into a classified `500
"authentication error"` (`lakehouse-auth/src/error.rs` `From<AuthError>`).
Now a database hiccup during verification, or a corrupt stored hash, is
(a) reported to the user as a wrong password, and (b) counted as a throttle
failure — five transient errors lock a real user out. This also falsifies
the handoff's "no existing response changed".

Fix: `match` the result. `Ok(p)` → success path. `Err(AuthError::InvalidCredentials)`
→ `record_failure`, audit, `warn!("login failed")`, `401`. Any other `Err(e)`
→ return `e.into()` without touching the throttle. Add an assertion that
the old mapping holds (a unit test on the match helper, or a route test
that a non-credential error does not create a `login_throttle` row).

#### BLOCKER 2 — the throttle storage tests have never run anywhere, CI included

`lakehouse-auth/tests/throttle.rs` does not link `lakehouse-test-support`
(every other file in that directory has `use lakehouse_test_support as _;`,
e.g. `tests/session.rs:19`). Without it nothing starts Postgres or sets
`DATABASE_URL`, and all seven tests panic: `DATABASE_URL must be set:
EnvVar(NotPresent)`. CI does not set `DATABASE_URL` either, so these would
have been red on the PR. The concurrency acceptance test is among them.

Reviewer experiment (local, not committed): with that one line added, all
seven pass (`cargo test -p lakehouse-auth --test throttle` → 7 passed). So
the SQL is right; the fix is the one line.

#### BLOCKER 3 — the cleanup tests cannot pass, and miss the plan's cases

`lakehouse-auth/src/cleanup.rs` `mod tests` — all seven fail when run
(same `DATABASE_URL` panic; fixing that exposes the rest):

- `INSERT INTO session (id, session_hash, …)` — the column is `token_hash`
  (`0019_auth.sql:77`). There is no `session_hash` anywhere.
- `app_user_id` and `service_identity_id` are fresh `Uuid::new_v4()` —
  both are `NOT NULL REFERENCES … ` (`0019_auth.sql:76,98`), so every seed
  fails on the foreign key.
- `gen_random_bytes` is `pgcrypto`; no migration enables it.
- `#[sqlx::test]` with no `migrations = "../../migrations"`, inside a crate
  that has no `migrations/` directory: the tables do not exist. Compare
  `tests/throttle.rs`, which gets this right.
- Plan T4 asks for one test seeding each kind **on both sides** of the
  cut-off and asserting **which rows remain**. Missing: a session expired
  inside the retention (must survive), a revoked session (both sides), a
  throttle row inside its window (must survive). The tests assert counts
  only.

Fix: move them to `lakehouse-auth/tests/cleanup.rs` with the migrations
path, use a user the migrations seed (as `tests/session.rs` does) and a
service identity created the way `tests/service_token.rs::seed_service_identity`
does (rule 4: reuse, do not re-invent), use `token_hash` with a bound
random hex value, and write the single table-driven test the plan
describes, asserting the surviving IDs. Keep the empty-tables test.

#### BLOCKER 4 — the docs advertise an off switch that does not exist

`README.md` (configuration table), `docs/OPERATIONS.md`, `CHANGELOG.md` and
`.env.example` all say `LOGIN_MAX_FAILURES=0` disables throttling. The code
(correctly, per decision "No off switch") treats `0` as unset and uses `5`.
An operator following the docs believes the throttle is off when it is on;
worse, the docs contradict a signed-off security posture (principle 2:
never fabricate). Fix the four docs: zero, negative or unparseable falls
back to the default; the throttle cannot be turned off. Do **not** change
the code.

#### SHOULD-FIX 1 — one mistake after a lock expires re-locks at once

`throttle::record_failure`: the lock is 300 s, the window 900 s. When the
lock expires inside the window, `failures` is still ≥ 5, so the next single
failure re-locks for another 5 minutes and writes another
`auth.login_locked` row. A user who waits out the lock and mistypes once is
locked again. This is a plan gap (the plan said "when the count reaches
`max_failures`" without saying what happens after expiry), corrected here:
**a failure on a key whose lock has expired starts a new window at 1.** In
the SQL, treat `login_throttle.locked_until IS NOT NULL AND
login_throttle.locked_until <= $2` the same as "window passed" in all three
`CASE`s. Add a test: lock, let it expire (short lockout), one failure →
not locked, `failures = 1`. The plan's T2 text is updated below.

#### SHOULD-FIX 2 — the concurrency test does not test concurrency

`tests/throttle.rs::ten_concurrent_record_failure_calls_all_count` asserts
only "locked" and "≥ 1 call saw the lock". With `max_failures = 5`, a lost
update that counted only 5 of 10 still passes. Fix: use a policy with
`max_failures` above 10 (e.g. 100) and assert
`SELECT failures FROM login_throttle WHERE key_hash = $1` is exactly 10.

#### SHOULD-FIX 3 — T5 has no component test

Plan T5 acceptance: a component test — a `429` shows the wait message, a
`401` still shows the credentials message. None was added (`bun run test`
is 301, the baseline). There is no login-page test yet; add `src/features/auth/login-page.test.tsx`. While
there: the text is "Try again in about 5 minute(s)." — the plan's wording is
"about 5 minutes"; pluralise properly.

#### SHOULD-FIX 4 — `docs/OPERATIONS.md` misses what the plan asked for, and says untrue things

Missing (plan T6): how an operator clears a lock by hand
(`DELETE FROM login_throttle WHERE key_hash = …`, and a one-line way to
compute the hash, e.g. `printf '%s' 'user@example.com' | sha256sum` after
trimming and lower-casing), and that the first admin can be locked out too.

Untrue, remove or correct:
- "skips a tick (with a logged warning) if the previous run has not
  finished" — the loop runs `purge` inline; there is no such warning. Same
  sentence in `CHANGELOG.md`, and "may skip a tick under load" in `README.md`
  and `SECURITY.md`.
- "`login_throttle` rows whose lockout expired more than
  `AUTH_RETENTION_DAYS` ago" — throttle rows go once unlocked and past the
  failure window; retention does not apply to them. The same error is in
  `cleanup.rs`'s `purge` doc comment ("at least `retention` old").
- "The throttle is in-process per deployment" — it is in Postgres.

`README.md`: the plan asked the cleanup bullet to be replaced with what
stays true — **active service tokens do not expire**. Add that bullet.
`CHANGELOG.md`: the plan asked for two or three plain sentences a customer
can read first, then the detail.

#### SHOULD-FIX 5 — `#[allow(missing_docs)]` with an uncheckable reason

`ThrottlePolicy` and `PurgeCounts`: "field names are self-documenting" is
not a checkable reason (`AGENTS.md`). Write a one-line doc on each field
and drop the `allow`.

#### Nits (fix if touching the file)

- `0055_login_throttle.sql` has no trailing newline.
- `docs/core/BACKLOG.md` is edited on this branch (the plan says not to);
  the change is identical to what `main` already has from #65, so it
  disappears when `main` is merged in.

#### Checked and correct

- `429` built only from fixed text and the lock time; nothing about the
  account reaches it. `401` path still returns `ApiError::invalid_or_expired()`.
- A locked key returns before `authenticate`: no Argon2 cost.
- `record_failure` is one `INSERT … ON CONFLICT DO UPDATE`; all values
  bound; the interval strings are built from integers and bound, not
  spliced into SQL.
- Throttle storage errors use `?` → `AuthError::Database` →
  `"authentication error"`; no fall-through to an unthrottled login.
- Audit row: `auth.login_locked`, `login_key`, 12 hex characters, no
  principal, no email or password; best-effort write.
- Cleanup `DELETE`s match the plan; none touches a live session or an
  active credential (`revoked_at < cutoff` is false for `NULL`).
- Config fallback rejects zero, negative and garbage; fields are in `Debug`.
- `Retry-After` is set only for the new variant; no other response changes.
- `tests/login_throttle.rs` covers all five T3 acceptance bullets.

#### Verification by the reviewer

Run on `5754a1e` in a fresh clone, Docker available, foreground:

- `cargo test -p lakehouse-auth --no-fail-fast` — lib: **54 passed, 7
  failed** (all of `cleanup::tests`); `tests/throttle.rs`: **0 passed, 7
  failed** (`DATABASE_URL must be set`); the other six binaries: 21, 7, 5,
  7, 8, 9 passed, 1 ignored.
- `cargo test -p lakehouse-api --test login_throttle` — **5 passed, 0
  failed**.
- Experiment above: `tests/throttle.rs` plus the harness import — 7 passed.
- Trial merge of `main` (`6e12254`) into the branch: one add/add conflict in
  this plan file; `main`'s copy is identical to the branch's original, so
  the branch's version is the resolution. No other conflict.

*Not verified by the reviewer:* full workspace clippy and test, the
TypeScript block (typecheck, lint, test), the compose line. These run once
on the fixed branch before merge, not now: the branch has blockers.

#### What the developer does next

Fix BLOCKERs 1–4 and SHOULD-FIXes 1–5, one commit each, citing
`PR review BLOCKER n` / `SHOULD-FIX n` at the fix site and in the commit
body (rule 13). Merge `main` in first (or merge the reviewer's branch that
carries this review). Then run the **DB-backed** tests, not only
`--no-run`: if Docker is unavailable on your machine, say so and the
reviewer runs them. End the report with the literal output of
`git status --short`, `git log --oneline origin/main..HEAD` and
`git ls-remote --heads origin feat/login-throttle-session-cleanup`.

### Review 2 — planner, 2026-10-03, on `0456d5f` merged with `main` `b81b3a1`

Review 1 above was written earlier the same day; the reviewer's push failed
then, so it lands on the branch together with this one. Correction to
review 1, SHOULD-FIX 4: "skips a tick if the previous run has not finished"
**is true** — the loop uses `MissedTickBehavior::Skip`. Only "(with a
logged warning)" is untrue.

Reality check: `656d68c`, `2de51bf`, `0456d5f` are on `origin`. `main` gained
#66 (pipelines; no auth files, no migration) — merged in, the plan file is
the only conflict, resolved as in review 1. `0055` is still free.

#### Closed

- BLOCKER 1 — fixed. `Err(AuthError::InvalidCredentials)` alone counts and
  renders `401`; any other error is the classified `500`, counted nowhere.
  The new route test (rename `auth_identity` mid-login) passes.
- BLOCKER 2 — fixed. `tests/throttle.rs` links the harness; its tests run.
- BLOCKER 4 — fixed in all four files.
- SHOULD-FIX 2, 5 — fixed. SHOULD-FIX 3 — test added (see below for the
  wording).

#### BLOCKER 5 — after the first lock expires, the key can never lock again

Introduced by the SHOULD-FIX 1 change in `throttle::record_failure`. The
reset predicate is `window passed OR locked_until < now`, but on a reset
the `locked_until` assignment falls to `ELSE login_throttle.locked_until`,
so the **expired** lock time stays on the row. Every later failure matches
`locked_until < now` again and resets the count to 1; with
`max_failures > 1` the key never reaches the limit. An attacker triggers
one lock, waits it out, then guesses without limit. The developer's own new
test `a_failure_after_the_lock_expires_starts_the_count_fresh` catches it
and **fails** (`throttle.rs:194`, "the second failure after expiry must
lock again") — it was never run.

Fix: in the `locked_until` `CASE`, add a branch before the `ELSE`:
`WHEN <the same reset predicate> THEN NULL`. Reviewer experiment (local,
not committed): with exactly that branch, `cargo test -p lakehouse-auth
--test throttle` → **8 passed**.

#### BLOCKER 3 — still open: the cleanup tests still do not run

`cleanup.rs` `mod tests` gained the migrations path and a correct survivor
matrix, but still has no `use lakehouse_test_support as _;`. Both tests
panic `DATABASE_URL must be set`, exactly as in review 1. Reviewer
experiment: with that one line inside `mod tests`, `cargo test -p
lakehouse-auth --lib cleanup` → **2 passed**. (Moving them to
`tests/cleanup.rs`, as review 1 suggested, works too.)

#### SHOULD-FIX 6 — leftovers from review 1

- `docs/OPERATIONS.md:653` — drop "(with a logged warning)"; no warning is
  logged.
- `README.md` "Status / Known limitations" — the bullet the plan asked for:
  active service tokens never expire (OPERATIONS.md says it; README does
  not).
- `login-page.tsx` — "about 5 minute(s)": write "minutes" (and "1 minute"),
  and update `login-page.test.tsx`, which now asserts the wrong wording.

#### Nit

- `docs/OPERATIONS.md` unlock SQL: "mirrors exactly" overstates it.
  `trim(' ' from …)` strips only spaces where `str::trim` strips all
  whitespace, and `lower()` follows the database collation. Say "for an
  ordinary address".
- Review 1 asked for one commit per finding; this round came as three.
  Keep to one per finding next round.

#### Verification by the reviewer (Docker available, foreground)

- `cargo test -p lakehouse-api --test login_throttle` — **6 passed, 0
  failed**.
- `cargo test -p lakehouse-auth --test throttle` — **7 passed, 1 failed**
  (BLOCKER 5).
- `cargo test -p lakehouse-auth --lib` — **54 passed, 2 failed** (BLOCKER 3).
- The two experiments above — 8/8 and 2/2.

*Not run:* full workspace clippy and test, the TypeScript block, compose.
They run once on the next round, before the PR.

#### Next round

Two one-line BLOCKER fixes and SHOULD-FIX 6, one commit each, cited at the
fix site. Then **run** `cargo test -p lakehouse-auth` and `cargo test -p
lakehouse-api --test login_throttle` — every test file this branch adds
must be reported with its pass count, not `--no-run`. If Docker is not
available, say so and stop; the reviewer runs them.

