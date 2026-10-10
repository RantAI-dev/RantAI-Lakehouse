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

## 8. Review (planner appends findings per PR)
