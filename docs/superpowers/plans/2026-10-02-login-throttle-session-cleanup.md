# Login throttling and session cleanup — Implementation Plan

**Status:** not started. Written 2026-10-02 by the planner (Claude Opus) for
a developer agent, under the role split in `AGENTS.md`.

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

_Empty._

## 8. Review (planner appends)

_Empty._
