# ADR 0016 — change capture per connector, on Kafka Connect

- **Status:** Accepted, 2026-10-29: the slice 0 measurement met every target
  in two runs (`docs/plans/SRC-4-RESULT.md`, CI runs 40816001350 and
  40817900374; worst p95 40.3 s against 120 s; Bronze equal to the source
  after 90 changes and after 108,000, nothing missing or duplicated, in both
  load modes). Was Proposed until then (`SRC-4` decision 13)
- **Phase:** Data roadmap phase 4, `SRC-4`
- **Date:** 2026-10-29
- **Supersedes:** ADR 0007's "no dynamic per-connector provisioning" (its
  rendering of a connector row into settings stays; what runs them changes)

## Context

Change capture today is one Debezium Server (`debezium-server-iceberg`)
declared in `docker-compose.yml`, with hand-written properties reading one
PostgreSQL table into Iceberg. A user who creates a change-capture connector
in the console gets a registry row and a rendered settings file, and nothing
reads their database. Debezium Server runs one source per process, so
"one per connector" means one container per connector, and nothing in the
deployment may start containers (that needs the Docker socket: a host
takeover for whoever reaches the service holding it).

The pipeline also writes straight to Iceberg in the sink's own upsert mode,
keeping deletes as rows marked `__deleted`. That skips the load modes every
other source has (`SRC-10`: current row per key, or history), the run
records, the alerts (`SRC-7`) and the schema change handling (`SRC-8`).

`SRC-4` needs change capture for PostgreSQL, MySQL, SQL Server, Oracle and
MongoDB, created by the user in the console, with no operator step
(product owner, 2026-10-29, decision 1).

## Decision

1. **One Kafka Connect service** (Debezium's distribution, pinned by digest)
   in the deployment holds every change-capture connector. The API adds,
   pauses, resumes and removes a connector through Kafka Connect's REST API
   when the user creates, pauses or deletes it. Kafka Connect is reachable
   only on the internal network; the API is its only client.
2. **One Kafka broker** (KRaft, single node in the template) carries the
   change topics, one per source table, plus Kafka Connect's own config,
   offset and status topics. Offsets live in Kafka, not in files.
3. **A Dagster job, the applier, writes changes to Bronze** in small batches
   through the same writer batch loads use (`adapters/sink.py`), so the
   `SRC-10` load modes apply: current row per key (a delete removes the
   row; each batch applied in one `pyiceberg` transaction, delete then
   append, as the history mode already writes, because `dlt`'s merge into
   Iceberg cannot delete) or history (every change kept with its operation
   and source commit time). The table is created and evolved through the
   batch writer's helpers. It commits its Kafka
   offsets only after the Bronze commit, so a crash replays rather than
   loses; the merge on the key makes a replay harmless.
4. **Secrets** reach Kafka Connect through a config provider that reads the
   connector's `secretRef` at start, never as a literal in the connector's
   JSON (which Kafka Connect stores in a topic and returns from its API).
   The mechanism is settled in slice 2 against ADR 0002.
5. **The fixed pipeline goes**: `debezium-server` and its sidecar are removed
   once PostgreSQL change capture runs on this path and `g4-cdc` passes on it.

## Consequences

- The deployment grows by a broker and a Kafka Connect worker: measured at
  1.2 to 1.3 GiB peak together in slice 0 (broker 580-637 MiB, worker
  632-735 MiB), with one connector and two tables; more connectors were not
  measured.
- A change reaches Bronze in under a minute at p95 (measured 18 to 40 s, the
  applier's 30-second sensor interval dominating), against the 2-minute target
  (decision 5); the fixed pipeline did it in 20 s without the load modes.
- Lag can be measured honestly in seconds: the applier knows the source
  commit time of the last change it applied, and Kafka knows the backlog.
- Kafka Connect's REST API can run arbitrary connector classes; it must
  never be exposed. The API holds an allowlist of the five Debezium classes.

## Rejected

- **One Debezium Server per connector, started by an operator** from a
  rendered compose fragment (`ops/debezium/render_compose.py`): works with
  today's code, but every connector waits for an operator. Kept as the
  fallback if slice 0 fails.
- **A supervisor that starts Debezium Server containers**: needs the Docker
  socket.
- **The Iceberg Kafka Connect sink** writing Bronze directly: skips the load
  modes, run records and alerts, as the fixed pipeline does today.
