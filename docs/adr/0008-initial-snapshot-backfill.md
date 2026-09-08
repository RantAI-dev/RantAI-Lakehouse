# ADR 0008 — initial snapshot/backfill for large tables

- **Status:** Accepted
- **Phase:** P5
- **Date:** 2026-09-01

## Context

A brand-new CDC connector faces a chicken-and-egg problem: streaming
logical replication only sees rows changed *after* the replication slot
was created, so a table with existing rows needs its current state
captured once, up front, before streaming picks up from there — Debezium
calls this the "initial snapshot." The plan names this as its own P5
deliverable (ADR 0008) separately from the CDC streaming path itself,
because a naive snapshot (`SELECT *` inside one transaction, one lock)
does not scale to a large production table without holding a long lock
or pinning WAL for the whole scan.

## Decision — Debezium's own snapshot mechanism, unmodified, is P5's answer

**This build does not implement a custom backfill job.** Debezium's
embedded snapshot phase — measured directly in `docs/plans/P5-RESULT.md`'s
end-to-end run (`Snapshot step 1` through `Snapshot step 7`, "Finished
exporting 3 records," then "Snapshot completed" before streaming began) —
already does exactly what ADR 0008 needs, and reimplementing it as a
separate Dagster/dlt job would duplicate a mechanism this build already
gets for free by using Debezium Server at all:

- **Snapshot then stream is automatic and ordered.** `debezium.source.
  snapshot.mode` (default `initial`) runs the snapshot before the
  connector's offset is created, and streaming picks up from the exact LSN
  the snapshot's transaction observed (`Read xlogStart at 'LSN{...}'`,
  measured in P5-RESULT.md's log) — no gap, no manual coordination between
  "backfill job" and "streaming job" the way a hand-rolled two-phase
  pipeline would need.
- **Lock scope is already minimal, not `LOCK TABLE ... EXCLUSIVE` for the
  whole scan.** Debezium's Postgres connector's default `snapshot.locking.
  mode` (`none` for the common case where the publication already exists,
  as this build's connectors always create it via
  `render_debezium_properties`'s `publication.autocreate.mode=filtered`,
  or briefly for schema consistency otherwise) does not hold a table-level
  lock for the data-copy phase — only (if at all) for capturing table
  structure. A hand-rolled backfill using a naive long-running transaction
  would have needed to reinvent this.
- **Chunking a genuinely large table is Debezium's `incremental` snapshot
  mode**, not a bespoke Dagster job: `debezium.source.snapshot.mode=
  incremental` (or `blocking`) breaks the initial copy into primary-key-
  ordered chunks, resumable and pausable, using Debezium's own
  watermarking mechanism — again, a mechanism this build gets by using
  Debezium Server rather than reimplementing.

## What this means concretely for a large-table connector

- **Default (`snapshot.mode=initial`)** is what `render_debezium_properties`
  ships for every connector today — appropriate for tables small enough
  that one initial copy (however long it takes) is acceptable before
  streaming begins. This matches G4's demo scale and is the safe default:
  it is Debezium's own default, not a value this build chose to be
  permissive.
- **A large source table** (the case this ADR is actually about) sets
  `debezium.source.snapshot.mode=incremental` at connector-registration
  time — a config-value decision, not a code change, since
  `render_debezium_properties` already takes a full properties body shape;
  extending it with a `snapshot_mode` field on `DebeziumSourceSpec` (a
  one-line addition when a real large-table connector needs it) is
  deliberately NOT done speculatively in P5, matching this build's
  standing rule of not building for a case nothing has asked for yet.
  Recorded here so the extension point has a name and a settled design
  when that connector arrives, rather than being redesigned from scratch.
- **WAL retention during a long incremental snapshot** is exactly R5's
  concern (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`'s risk register) — the
  slot exists and pins WAL from the moment it is created, whether the
  connector is still snapshotting or already streaming. This is why P5's
  slot-lag/WAL-retention metrics (`replication_slot_check_job`, every 15
  minutes) are not gated on "streaming has started" — they check every
  logical slot on the source unconditionally, snapshot-phase or not.

## What this ADR does NOT do

- Does not implement a `blocking`/`incremental` snapshot connector in this
  phase — no real large-table connector exists yet to validate it against,
  and Debezium's own incremental-snapshot mechanism is already a stable,
  documented feature, not something this build needs to prove works (unlike
  the two genuinely novel integrations this phase DID measure: equality
  deletes and the REST-catalog write path).
- Does not add a `snapshot_mode` field to `DebeziumSourceSpec` speculatively
  — see above.

## Consequences

- No new Rust/Dagster code ships against this ADR; it is a decision record
  (use Debezium's own mechanism, do not build a parallel one) plus the
  concrete config knob (`snapshot.mode`) a future large-table connector
  turns when it exists.
- `docs/plans/P5-RESULT.md`'s measured snapshot-then-stream sequence is
  this ADR's verification: the mechanism ADR 0008 relies on was observed
  working end to end, not assumed from Debezium's documentation alone —
  consistent with this build's standing "measure, don't trust the brief"
  rule.

## Verification

`docs/plans/P5-RESULT.md`'s "(B)" section and G4
(`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`'s P5 acceptance criteria) both
exercise the default `snapshot.mode=initial` path end to end: a source
table with pre-existing rows, captured once at connector start, followed
immediately by streamed `UPDATE`/`DELETE`/`INSERT` — the same sequencing
this ADR's decision depends on.
