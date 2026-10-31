# Install and update in one step, online or air-gapped

| | |
| --- | --- |
| Module | Delivery |
| Backlog | `REL-2` |
| Kind | Task |
| Spec | `docs/core/specs/rel-2.md` (four decisions and the *(proposed)* numbers await the product owner) |
| Status | Planned |
| Priority | P1 |
| Owner | In the base. |
| Acceptor | In the base. |
| Plan | `docs/superpowers/plans/2026-10-31-rel-2-one-step-install-and-update.md` |
| Started | 2026-10-31 |
| Shipped | |
| Evidence | |

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

## Decisions

See the spec's "Decisions the product owner makes", and the plan's D5 to D12.

## Limits

One host. The stack stops during an update. Linux x86_64 first.

## Acceptance checklist

The rows are in the spec: `REL-2-AC1` to `REL-2-AC7`.

## Not included

Kubernetes, multi-host, zero-downtime, Windows and macOS hosts, unattended updates.

## Asking the assistant

No. This is an operator tool.
