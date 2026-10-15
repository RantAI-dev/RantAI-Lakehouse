# SEC-10 Alert webhooks cannot reach internal addresses — Implementation Plan

**Status:** decisions signed 2026-10-10, not started. Written by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/webhooks-cannot-reach-internal-addresses.md`.
Spec: `docs/core/specs/sec-10.md`.

**Base:** branch `fix/sec-phase-0` at `4f99bb7` (phase 0 branch; `SEC-13`
and `SEC-11` are already on it). Read `upstream_error.rs` from `SEC-11`
first: failed deliveries reported to a caller go through it.

**Goal:** the webhook sender only ever connects to an address that passed
the same check the connector probe uses, never follows a redirect, and an
administrator can list internal hosts that are allowed.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Blocked set | Exactly the set `connector_probe::is_blocked_ip` refuses (it was widened by `SEC-15`). One implementation, shared; not a second copy. |
| Redirects | Not followed. A 3xx is a failed delivery with a fixed reason. |
| Pinning | Resolve the host once, check every returned address, connect to a checked address. The HTTP client must not resolve the name again. |
| Allowlist | One deployment setting, empty by default, listing internal hosts webhooks may reach. Reuse the mechanism the connector probe already has for internal hosts (`internal_hosts`); do not invent a second format if that one fits. If it does not fit webhooks (it is scoped to connectors), stop and tell the planner what it is before designing another. |
| When | At rule save, at "test", and at every send. |
| Schemes and ports | `http` and `https` only, as today. No port restriction beyond what the shared check already applies. |
| Messages | Fixed text of ours: "webhook address is not allowed" for a refused target, "webhook redirect not followed", "webhook host could not be resolved", "webhook HTTP <status>" (already ours). Never the HTTP library's error text, in a response or in a stored delivery record. |

## 2. What exists today (anchors, verified at `4f99bb7`)

- `rust/crates/lakehouse-notify/src/lib.rs:83` `send_webhook(client, url, title, text)`: prefix check, `client.post(url)`, and `Err(err) => DeliverResult::err(err.to_string())` (library text into the delivery result).
- `rust/crates/lakehouse-alerts/src/lib.rs:394`: the same prefix check when a rule is normalised; the alert and digest runners take `http: &reqwest::Client` (`:1233`, `:1370`, `:1414`, `:1450`, `:1495`).
- `rust/crates/lakehouse-api/src/routes/alerts.rs:515`, `:1301`, `:1360` build `reqwest::Client::new()` (default redirect policy: follows up to 10).
- `rust/crates/lakehouse-api/src/connector_probe.rs`: `resolve_checked(host, port, internal_hosts)` (`:1030`), `is_blocked_ip` (`:849`), and the allowlist it takes (`internal_hosts`). They live in the API crate; `lakehouse-notify` and `lakehouse-alerts` are below it and cannot call up.

## 3. Tasks

One commit per task.

### T1 — One shared address check

Make the address check and the checked resolution usable from the notify
layer without duplicating them: either move `is_blocked_ip` (and its `v4`
helper and tests) and the resolution step into a crate both the API and
`lakehouse-notify` depend on (`lakehouse-core` is the usual home), with
the connector probe re-exporting or calling it so its behaviour and tests
are unchanged; or pass the check into the sender as a small trait or
closure the API crate implements with `resolve_checked`. Choose the one
that leaves a single implementation and say why in the handoff. No change
in what the connector probe refuses: its existing tests stay green
unchanged.

*Accept:* the moved tests pass in their new home; connector probe tests
unchanged and green.

### T2 — The sender

`send_webhook` (or a new function replacing it; update every caller):
parse the URL; refuse a scheme other than `http`/`https`, a URL with
userinfo, and an empty host; resolve the host through T1 with the
allowlist; refuse if any resolved address is blocked and not allowed; build
the request so the connection goes to a checked address (for `reqwest`,
`ClientBuilder::resolve` / `resolve_to_addrs` for that host on a client
built for this send, or the equivalent the workspace's version offers;
verify against the version in `Cargo.lock`, do not guess); redirect policy
none; a sensible timeout if the callers do not already set one (state what
it was and what it is). An IP-literal host is checked directly. Map every
failure to the fixed messages in section 1.

*Accept:* unit tests with a local listener: a public-looking name mapped to
the listener through the test seam is delivered; `127.0.0.1`, `[::1]`,
`10.1.2.3`, `169.254.169.254`, `100.64.0.1`, an IPv4-mapped IPv6 form, and
a name resolving to one of them are refused before any connection (assert
the listener saw nothing); a 302 to anywhere is a failed delivery and the
second hop is never requested; an allowlisted internal host is delivered;
no result string contains library text (planted marker).

### T3 — Save and test paths

Rule normalisation in `lakehouse-alerts` (the `:394` check) and the
"test"/"send now" routes in `routes/alerts.rs`, the digest paths, and the
assistant's alert tools (`routes/ai/tools/alerts.rs`): refuse a disallowed
webhook when it is saved or tested, with the fixed message and a 400, not
only when it fires. Where saving cannot resolve a name (no network in a
unit path), say so in the code and keep the send-time check as the
guarantee. Replace the three `reqwest::Client::new()` call sites with the
sender from T2 so none follows redirects.

*Accept:* route tests: saving and testing each refused form answers 400
with the fixed message; a role without the alerts permission is still
refused first (`SEC-10-AC3`); the `SEC-11` guard still passes.

### T4 — The allowlist setting

Wire the setting from configuration to the sender (`${X:-}` in
`docker-compose.yml` and `.env.example`, empty default, with a why-comment;
no default host). Document its format where the connector allowlist is
documented.

*Accept:* a test that an entry allows exactly that host and no other; a
malformed entry stops the API at start with a config error, as other
settings do, rather than being ignored.

### T5 — Console and docs

If the console shows a webhook validation or delivery error, it shows the
fixed message (it should need no change; confirm). `CHANGELOG.md`
`[Unreleased]` under Security. `docs/OPERATIONS.md` (or wherever outbound
settings are described): the new setting and the redirect rule.

## 4. PR slicing

Commits T1…T5 on the phase 0 branch.

## 5. Out of scope (do not build)

SMTP; other outbound callers; retries; per-rule allowlists; changing what
the connector probe refuses.

## 6. Things the developer must verify, not assume

- Every caller of `send_webhook` and every place a webhook URL is accepted or stored (alerts, digests, notification channels if any, assistant tools, YAML import if dashboards or alerts can be imported).
- That the client really connects to the checked address: prove it with a test where the name's "real" resolution would differ from the pinned one.
- That an `https` webhook still verifies the certificate against the host name when the connection is pinned to an address.
- Whether stored delivery records with old library text are shown anywhere; do not rewrite history, but say so.
- A compose edit is proven by `docker compose --profile '*' config --quiet` at least; a real `up` is the reviewer's call on this shared machine, so say that it was not done.

## 7. Handoff (developer appends one entry per PR)

**SEC-10 — T1–T5 (developer, 2026-10-10).** Nothing committed (owner QA
first); the work is splittable per task by the file lists below.

*Files per task.*
- T1: `rust/crates/lakehouse-api/src/connector_probe.rs` (new
  `first_refused`, `check_addrs` now calls it; behaviour unchanged),
  `rust/crates/lakehouse-api/src/webhook_guard.rs` (new; also its `mod` line
  in `lib.rs` and `main.rs`), `lakehouse-notify` trait `TargetResolver` (in T2's file).
- T2: `rust/crates/lakehouse-notify/src/webhook.rs` (new), `.../lakehouse-notify/src/lib.rs`
  (`send_webhook` removed, `deliver` takes `&WebhookSender`, tests),
  `.../lakehouse-notify/Cargo.toml` (feature `test-support`).
- T3: `lakehouse-alerts/src/lib.rs` + `Cargo.toml` (dev-dep feature),
  `lakehouse-api/src/routes/alerts.rs`, `routes/pipelines.rs`,
  `routes/ai/tools/alerts.rs`, `routes/ai/tools/mod.rs`.
- T4: `lakehouse-api/src/config.rs`, `docker-compose.yml`, `.env.example`.
- T5: `CHANGELOG.md`, `docs/OPERATIONS.md`. Console: no change (confirmed: a
  save error shows `saveAction.error.message`, a delivery error shows `delivered.error`).

*T1 design chosen: the check is passed into the sender as a trait.*
`lakehouse_notify::TargetResolver` is implemented by
`webhook_guard::WebhookGuard` in the API crate, which calls
`connector_probe::first_refused` (the same `is_blocked_ip` +
`InternalHosts::permits` the connector dials use). One implementation, nothing
moved, the connector probe's tests untouched. Moving `is_blocked_ip` and
`InternalHosts` into `lakehouse-core` was rejected: `InternalHosts` has
`#[cfg(test)]` constants (`ALL`, `NONE`) that the API tests use and that
cannot be inherited across crates, and it would add `ipnet` to core.

*Allowlist.* `InternalHosts` is CIDR-based (not host names) and its setting
(`CONNECTOR_PROBE_ALLOWED_CIDRS`) is connector-scoped, with an allow-all
flag. I judged the FORMAT fits (parser, never-listed rules reused as is) but
not the setting: new `WEBHOOK_ALLOWED_CIDRS`, same parser and type, no
allow-all. Planner: this is the one place I read "stop if it does not fit"
loosely; say if you wanted it shared.

*Every place a webhook URL is accepted, stored or sent.*
- Accepted and stored: `POST`/`PUT /api/alerts` (`routes::alerts::create`/`update`)
  and the assistant's `create_alert_rule`/`update_alert_rule`, all through
  `lakehouse_alerts::save_rule`, which now takes `&WebhookSender`, and after
  `normalize_input` (still the sync scheme check) calls `check`: 400/tool error
  "webhook address is not allowed" (or "webhook host could not be resolved").
  No YAML/dashboard import, notification-channel table or other writer exists
  (grep of `save_rule` / `AlertRuleInput`).
- Sent (all via `deliver` -> `WebhookSender::send`, checks again every time):
  `/api/alerts/run` (rules, digests, freshness, late pass), `run_failed_event`
  and the slow/volume-drop event handlers in `routes/pipelines.rs`, the
  assistant's `run_alert_rule`. The three `reqwest::Client::new()` in
  `routes/alerts.rs`/`ai/tools/alerts.rs` and two in `routes/pipelines.rs` are gone.
- "Test" / "send now": there is no separate test route; it is
  `/api/alerts/run?id=` (and the assistant tool), which answers 200 with the
  per-rule result, whose `delivered.error` is the fixed message. I did NOT
  turn that into a 400 (the response is a list of results, and the cron
  caller needs the rest of the run). Deviation from T3 text; planner to decide.

*Behaviour changes to tell the owner.* A 10 s total / 5 s connect timeout
(there was none); the system proxy is ignored (a proxy resolves the name, which
defeats the pin) so a deployment that reaches webhooks only through
`HTTPS_PROXY` stops working; a fresh `reqwest` client is built per delivery.

*Pinning and TLS.* `ClientBuilder::resolve_to_addrs(host, addrs)` (reqwest 0.12,
same call the SEC-15 REST probe uses) for domain hosts; IP literals are
checked and dialled directly. Proved by test: `.test` names exist nowhere in
DNS yet a delivery to one succeeds only through the pinned address (notify
`a_named_webhook_connects_to_the_approved_address_not_to_dns`, API
`a_name_resolving_to_a_checked_address_is_delivered_there`). *Not verified by
a test:* that an `https` webhook still checks the certificate against the host
name (no offline TLS fixture); this rests on reqwest/rustls deriving the
server name from the URL and `resolve_to_addrs` only replacing the address
lookup, as the SEC-15 notes state. Worth one real HTTPS webhook at QA.

*Stored delivery records.* None: `alert_instance.delivery_status` is never
written, and the delivery result appears only in the `/api/alerts/run`
response, so no old library text is stored.

*Test seams.* `lakehouse_notify::MappedResolver` (cfg(test) or feature
`test-support`, enabled only from alerts' `[dev-dependencies]`); the guard's
`#[cfg(test)]` name map; `Config.webhook_test_allow_all`, true only if
`cfg!(test)` and `WEBHOOK_TEST_ALLOW_ALL=true` (always false in a service build).

*Existing tests changed.* `lakehouse-notify` webhook tests (client type
`reqwest::Client` -> `WebhookSender`; same assertions); 17 `lakehouse-alerts`
tests (`reqwest::Client::new()` -> `test_webhook_sender()`, 8 `save_rule`
calls gained the sender argument); `routes/alerts.rs` late-pass tests and
`ai/tools/alerts.rs` tests (sender argument); `routes/pipelines.rs`
`run_finished_event_route::state_for` sets `WEBHOOK_TEST_ALLOW_ALL` because
that test delivers to a loopback `wiremock` (the old behaviour was by
design allowed). No assertion weakened.

*Unused dependency.* `lakehouse-alerts` no longer uses `reqwest`; I left it in
`Cargo.toml` so `Cargo.lock` stays unchanged under `--locked`. Remove it in a
follow-up if wanted.

*Commands run (foreground, from `/home/hv/lakehouse-uiux/rust`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`).*
- `cargo fmt --check`: clean (after `cargo fmt`; all hunks were mine).
- `cargo clippy -p lakehouse-notify -p lakehouse-alerts -p lakehouse-api --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo test -p lakehouse-notify -p lakehouse-alerts --locked`: notify 14 passed, alerts 70 passed.
- `cargo test -p lakehouse-api --lib --locked`: 1516 passed, 0 failed, 1 ignored (includes `webhook_guard` 6, `routes::alerts::tests::webhook_targets` 1, config `webhook_allowed_cidrs` 1, `connector_probe`/`internal_hosts` unchanged).
- `cargo test -p lakehouse-api --locked --test sec11_guard --test route_auth --test security_regressions`: 4 + 30 + 10 passed.
- `cargo test -p lakehouse-api --locked --test test_connection_route --test upstream_errors`: 3 + 6 passed.
- `docker compose --profile '*' config --quiet`: ok. No `up` was done.
- TypeScript untouched, so `bun` checks not run. `Cargo.lock` unchanged. Test executables over 20 MB deleted from the target `deps` afterwards.

*Not verified.* A real `https` webhook (certificate check); a real DNS name
resolving to a private address (the guard's name map stands in); the stack with
the new env var (`docker compose up`); the `SEC-10-AC3` permission refusal was
not re-tested beyond the existing `route_auth` suite (the policy layer runs
before the handlers; no route or `POLICY_TABLE` change).

## 8. Review (planner appends findings per PR)

### SEC-10 — T1–T5 (reviewer, 2026-10-10)

Reviewed against the plan and run on a rebuilt API.

- **Accepted deviations.** (1) The allowlist is its own setting,
  `WEBHOOK_ALLOWED_CIDRS`, in the connector allowlist's format but not
  shared with it and with no allow-all switch; the plan said to stop if the
  connector mechanism did not fit, and the developer judged the format fit
  and the setting did not. The reviewer agrees with the outcome. (2) "Run
  now" answers 200 with the fixed message in that rule's result rather
  than a 400, because the scheduled caller reads a list of results; only
  saving answers 400. Both are on the feature page.
- **Checked specifically:** `WEBHOOK_TEST_ALLOW_ALL` is read behind
  `cfg!(test)` in `config.rs`, so it is a compile-time `false` in a service
  build; it appears in no compose file or example env.
- **No open BLOCKER.**
- **SHOULD-FIX, not done:** two refusals differ only by a full stop
  ("invalid webhook URL" and "invalid webhook URL."). Cosmetic.

Verified by the reviewer on the final tree: `cargo fmt --check` clean;
`cargo clippy -p lakehouse-notify -p lakehouse-alerts -p lakehouse-api
--all-targets --all-features --locked -- -D warnings` clean;
`lakehouse-notify` 14, `lakehouse-alerts` 70, `lakehouse-api --lib` 1516
(1 ignored), `sec11_guard` 4, `route_auth` 30, `security_regressions` 10
passed; `docker compose --profile '*' config --quiet` exit 0.

On the running API, `POST /api/alerts` with a webhook to `127.0.0.1`,
`localhost`, `[::1]`, `10.1.2.3`, `169.254.169.254`, `100.64.0.1` and an
IPv4-mapped IPv6 private address each answered 400 "webhook address is not
allowed" and stored nothing; a URL with userinfo and an `ftp` URL answered
400; a public `https` URL was stored. The product owner repeated the
refusal and the public save in the console on 2026-10-10.

*Not verified:* a delivery to a real public webhook (so the `https`
certificate check with a pinned address rests on the library); a redirect
from a public URL on a running system; a name that really resolves to a
private address; the compose stack started with the new variable;
`cargo test --workspace`.

Carried forward, for the owner: webhooks no longer use a system proxy, so
a deployment whose only way out is `HTTPS_PROXY` cannot deliver to public
services; deliveries time out at 10 seconds.

