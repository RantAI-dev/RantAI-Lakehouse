# Install and update in one step, online or air-gapped

| | |
| --- | --- |
| Module | Delivery |
| Backlog | `REL-2` |
| Kind | Task |
| Spec | `docs/core/specs/rel-2.md` (four decisions and the *(proposed)* numbers await the product owner) |
| Status | In Acceptance (all five slices merged; air-gap proofs #693 and #711) |
| Priority | P1 |
| Owner | In the base. |
| Acceptor | In the base. |
| Plan | `docs/superpowers/plans/2026-10-31-rel-2-one-step-install-and-update.md` |
| Started | 2026-10-31 |
| Shipped | |
| Evidence | #295 (slices 1–5); #693 (`REL-12`, host `OUTPUT` air-gap proof); #711 (`REL-16`, bundled init image and container `DOCKER-USER` proof); `OPS-2` runbook and completeness guards |

## Problem

- Installing takes about ten steps from source, including a 911-line settings file and a
  console started by hand.
- Updating means rebuilding, with no automatic backup and no way back.
- An air-gapped site has no supported path at all.
- The connector-secrets volume is in no backup.

## What the user can do when this is done

1. Install with one command on a Linux host that has Docker, with no file to edit.
2. Install on a host with no network from one signed file, verified offline.
3. Be told before installing what is missing on the host (memory, disk, Docker version, ports).
4. Update with one command. A backup is taken first, and a failed update rolls itself back.
5. Go back to the previous release with one command.
6. Back up and restore on demand, to local disk or S3.
7. See the version in the console, and be told when the console and server versions differ.

The installer writes the `.env` itself: a generated value for every secret,
`COMPOSE_PROFILES=dagster`, and `CATALOG_TENANT_ID` naming the group tenant, so
a fresh install runs its scheduled work without a hand edit (`REL-10`).

## Decisions

See the spec's "Decisions the product owner makes", and the plan's D5 to D12.

## Limits

One host. The stack stops during an update. Linux x86_64 first.

## Air-gapped install, update and rollback (`OPS-2`)

A host with no internet installs from one signed bundle (`lakehousectl install --bundle`),
moves to a newer release with `lakehousectl update --bundle` (a local backup first and an
automatic rollback if the stack is not ready), and goes back with `lakehousectl rollback`.
The bundle carries every image the rendered Compose file runs, so `docker load` needs no
registry, and nothing installs a package when a container starts.

The two proofs in the release workflow are the evidence, not a claim:

- **#693 (`REL-12`)**: both proof jobs put a real iptables REJECT in the host's `OUTPUT`
  chain for uid 0 and keep the must-fail `docker pull`; a runner that can still reach a
  registry now fails the proof instead of silently fetching an image.
- **#711 (`REL-16`)**: the five init containers that used to run `apk add` at start now
  share one bundled `rantai-lakehouse-init` image, and the proof also blocks container
  egress at `DOCKER-USER`, so a first start with no package mirror and no registry is what
  is exercised, while the stack still reaches `/health/ready`.

`OPS-2` ("Install with no internet access") is in acceptance against that evidence. The
operator steps are in `docs/OPERATIONS.md`, "Air-gapped installation (no internet access)";
`ops/release/test_ops2_airgap_completeness.py` holds the permanent guards (no runtime
package fetch or external download in the rendered release Compose, every image named by
the signed manifest, the runbook's presence, and a local-only pre-flight under
`--bundle`). The honest limits — optional external integrations and the profiles the
bundle does and does not carry — are written in that runbook.

## Acceptance checklist

The rows are in the spec: `REL-2-AC1` to `REL-2-AC7`.

## Not included

Kubernetes, multi-host, zero-downtime, Windows and macOS hosts, unattended updates.

## Asking the assistant

No. This is an operator tool.
