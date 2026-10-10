# Connection tests cannot reach internal addresses

Backlog `SEC-15`. Spec: [`../specs/sec-15.md`](../specs/sec-15.md).
Plan: `docs/superpowers/plans/2026-10-08-sec-15-probe-internal-addresses.md`.

## What the user can rely on

A connection test, a table discovery or the clean-up of a deleted connector
never dials an address inside the installation's own network unless the
operator has said that it may, and never tells the caller which address a
name resolved to.

## Decisions

The product owner asked on 2026-10-08 for the Phase 0 security items to be
fixed as their specs say. The spec's Target table is taken as signed. The
choices below are the planner's, where the spec leaves room.

| # | Decision | Choice | From |
| --- | --- | --- | --- |
| 1 | Default | Internal addresses are blocked when `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` is unset, in compose as in the code. Allowing them is an explicit setting, documented, with `CONNECTOR_PROBE_ALLOWED_CIDRS` as the narrower choice. | spec |
| 2 | Redirects | The REST test does not follow redirects. A redirect answer is reported as one, with a fixed message. | spec offers either; this is the simpler to prove |
| 3 | Pinning | The check returns the address it approved and the dial uses that address. A name is resolved once per dial. | spec |
| 4 | Which dials | Every dial the API makes to a connector's target: test, discovery, and the clean-up on delete. | spec names tests and delete; discovery shares the code |
| 5 | A dialer that cannot be pinned | If a client library gives no way to dial a chosen address, that connector type's test is refused with a fixed message while the block is on, rather than dialled unpinned. | planner |
| 6 | Messages | Fixed sentences. No resolved address, no resolver or driver text. The caller's own host name may be repeated back. | spec |
| 7 | Delete when the target is refused | The delete is refused with a fixed message; `force=true` still removes the row without dialling, as it does today when the clean-up fails. | planner |

## Limits

- An installation whose sources are on a private network must set the
  allow-list (or the allow-all switch) before any connection test works.
  The seeded demo connectors point at services inside the compose network,
  so on a fresh install their tests are refused until the operator opts in.
- With an address allowed, "connection refused" and "timed out" still differ
  in a test result, so a person who may test connectors can tell an open
  port from a closed one on that network. The spec's Target table does not
  cover it.
- A name that resolves to several addresses is refused if any is blocked;
  otherwise the first address the system ranks first is dialled and the
  others are not tried.
- The REST and S3 tests do not use a system proxy: a proxy would resolve the
  name itself and defeat the pin.
- The orchestrator's own dials are a separate guard with its own settings
  and are not changed here.
- TLS and what a server is sent are `SEC-14`.

## Acceptance checklist (product owner, on a running console)

- [ ] Every Target row of the spec works as written
- [ ] On a fresh compose install, a test against `127.0.0.1` is refused
- [ ] A REST test against a public URL that redirects to `169.254.169.254` is refused
- [ ] The refusal names no address
- [ ] With the allow-list set to the source's network, the same test runs
- [ ] Deleting a change-capture connector whose target is internal is refused, and `force=true` removes it
- [ ] A user without the permission is refused
