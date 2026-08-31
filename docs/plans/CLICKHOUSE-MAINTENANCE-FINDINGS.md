# ClickHouse 26.3 — Iceberg maintenance surface, measured

Evidence gathered while validating the 24.8 → 26.3 bump in P1a. Two items
in the build brief's "verified facts (ClickHouse docs, Aug 2026)" do **not**
hold against the running server. Both affect P4's maintenance chain, not P1.

Probed against `clickhouse/clickhouse-server:26.3` (`SELECT version()` →
`26.3.26.3`).

## Confirmed as briefed

`SELECT name FROM system.settings` — these exist:

| Setting | Tier |
| --- | --- |
| `allow_database_iceberg` | Beta |
| `allow_insert_into_iceberg` | Beta |
| `allow_experimental_insert_into_iceberg` | Beta |
| `allow_experimental_expire_snapshots` | Experimental |
| `allow_experimental_iceberg_compaction` | Experimental |

`ALTER TABLE … EXECUTE <verb>()` is a recognized grammar, dispatched
per storage engine. Against a MergeTree table it fails with:

```
Code: 48. DB::Exception: EXECUTE command 'expire_snapshots' is not
supported by storage MergeTree. (NOT_IMPLEMENTED)
```

That is an engine-dispatch rejection, not a parse failure — so the verb
exists and should reach an Iceberg table. Confirming it actually *works*
requires a real Iceberg table, which needs the full P1 stack. **Deferred to
a P4 acceptance item.**

Also present and useful: `iceberg_expire_default_max_snapshot_age_ms`
(default 432000000 = 5 days), `iceberg_expire_default_min_snapshots_to_keep`
(1), `iceberg_expire_default_max_ref_age_ms` — all Production tier. These
are the knobs for `expire_snapshots`.

## Correction 1 — `allow_iceberg_remove_orphan_files` does not exist

The brief lists it as one of three experimental gate settings. It is not in
`system.settings` on 26.3, and there is **no setting matching `%orphan%` at
all**.

The `remove_orphan_files` EXECUTE verb itself gets the same per-engine
NOT_IMPLEMENTED dispatch as `expire_snapshots`, so the verb plausibly
exists and simply is not setting-gated on this version. Unverified either
way until P4 runs it against a real Iceberg table.

## Correction 2 — `OPTIMIZE TABLE … MANIFEST` is a syntax error

This is the harder finding. It is step 4 of the briefed four-command
maintenance chain, and the grammar does not accept it:

```
Code: 62. DB::Exception: Syntax error: failed at position 24 (MANIFEST):
MANIFEST. Expected one of: ON, PARTITION, DRY RUN, FINAL, FORCE,
DEDUPLICATE, CLEANUP, INTO OUTFILE, FORMAT, SETTINGS, ParallelWithClause,
PARALLEL WITH, end of query. (SYNTAX_ERROR)
```

The full list of accepted `OPTIMIZE` keywords is in that error, and
`MANIFEST` is not among them. So the maintenance chain on 26.3 is three
commands, not four.

Note `DRY RUN` **is** accepted — that is the mechanism for P4's
"`dry_run` metrics surfaced in console" requirement.

## Consequences for P4

- The chain becomes `expire_snapshots` → `remove_orphan_files` →
  `OPTIMIZE` (Iceberg, experimental). Manifest rewriting is not available
  as briefed and needs either a different mechanism or an accepted gap.
- Combined with the already-known absence of bin-pack rewrite of small data
  files, **two** of the small-file mitigations assumed in the plan are
  unavailable in-engine. This raises the prior probability that G3 fails
  and the Trino-as-cron escape hatch is needed. Risk R2 in the plan is
  upgraded accordingly.
- Whether the verbs work at all on a catalog-registered Iceberg table is
  still unmeasured. That measurement is now an explicit P4 acceptance item
  rather than an assumption.

## Confirmed safe: the 24.8 → 26.3 bump itself

All seven `demo/clickhouse/*.sql` files apply cleanly (via
`clickhouse-client --multiquery`; note the HTTP interface rejects
multi-statement bodies, which is a harness detail, not a server change).
`cargo test --all-features` passes 723 tests across 28 targets.

**Caveat worth stating plainly:** the Rust test suite does *not* exercise
ClickHouse. `lakehouse-test-support` enables `testcontainers-modules` with
the `postgres` feature only, so no ClickHouse container is ever started. A
green `cargo test` is therefore not evidence about the bump — the demo-SQL
application above is.
