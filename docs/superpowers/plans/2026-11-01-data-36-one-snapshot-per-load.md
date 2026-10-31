# DATA-36: a streamed upload load leaves one Iceberg snapshot, not one per 32 MiB group

Backlog `DATA-36` (Task, Data, P2). Reassigned to the planner lane on 2026-11-01. Base: `main` at
`a33e3d81`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code disagrees, stop
and report it.

## What is wrong (confirmed)

- **Each group is its own snapshot.** In `_write` (`dagster/dispar_orchestrate/adapters/iceberg_stream.py:160-186`),
  one `table.transaction()` calls `transaction.overwrite` for the first group of a replace and
  `transaction.append` for each later group (`:175-178`). Each call produces its own snapshot,
  so a 2 GiB load adds about 64 snapshots, and a replace adds a DELETE snapshot first.
- **Every half-loaded state can be chosen.** The time-travel picker
  (`src/features/queries/time-travel-controls.tsx:116-118`) lists every snapshot.
- **The test missed it.** `test_iceberg_stream.py:51` counts metadata commits, not snapshots.
- **The same pattern** is in `adapters/sink.py:932-955` (`_replace_changing_types`).

## Decisions

- **D1. One snapshot producer per load, with memory still bounded per group.** Pinned versions are
  `pyiceberg==0.12.0` and `pyarrow==25.0.1` (`dagster/pyproject.toml:20`).
  - **Append:** one `transaction.update_snapshot(snapshot_properties=…).fast_append()` producer.
    For each group, write its data files with
    `pyiceberg.io.pyarrow._dataframe_to_data_files(table_metadata=transaction.table_metadata, write_uuid=producer.commit_uuid, df=group, io=table.io)`,
    and call `producer.append_data_file(f)` for each file.
  - **Replace:** one `update_snapshot().overwrite()` producer that deletes every data file of the
    current snapshot, then appends as above. That gives **one** OVERWRITE snapshot.
  - Keep the zero-row commit (`:180-186`).
  - `_dataframe_to_data_files` is private API. Pin it with a comment that names the pinned version,
    and add a test that fails loudly if the import breaks on an upgrade. If 0.12.0 lacks it, or a
    producer method, **stop and report**. The public fallback is to write Parquet per group and call
    `transaction.add_files` once.
- **D2.** The snapshot properties (author, `DATA-15`) stay as today: one set, on the one snapshot.
- **D3.** Apply the same change to `sink._replace_changing_types`.

## Task (one commit)

- **T1.** Make D1 to D3. Cite `DATA-36`.
- Tests in `test_iceberg_stream.py`, each also asserting a snapshot-count delta of 1 (use
  `lab.snapshots` or the table's history):
  - a multi-group replace (`:51`);
  - a multi-group append (`:74`);
  - a zero-row load (`:195`).
- Tests elsewhere:
  - the retype path in `test_typed_load.py:322` also leaves one snapshot;
  - the memory guard in `test_load_memory.py` still passes.
- *Accept:* `(cd dagster && python -m pytest dispar_orchestrate -q)`, the whole Dagster suite. It
  is pure Python, so it is cheap.

## Handoff (developer appends below)

### Handoff (developer, 2026-11-01)

**Commit:** `2a82a95f` — `fix(iceberg): a load leaves one snapshot, not one per group (DATA-36)`, on
`fix/data-36-one-snapshot-per-load` (base `b6f8b1d2`, the plan's own commit). One commit, as T1
says.

**What changed.** `dagster/dispar_orchestrate/adapters/iceberg_snapshot.py` (new) holds the
shared write: `write_one_snapshot(table, transaction, groups, replace=...)` writes every group of
a load into ONE snapshot producer — an `append` is one `fast_append`, a `replace` is one
`overwrite` that deletes every data file of the current snapshot and adds the load's own.
`adapters/iceberg_stream.py::_write` (D1) and `adapters/sink.py::_replace_changing_types` (D3)
both call it; the memory bound per group is unchanged (the groups are still written one at a
time, `FLUSH_BYTES` / `RETYPE_FLUSH_BYTES`).

**Commands run (foreground, on the final commit):**

| Command | Result |
| --- | --- |
| `cd dagster && python -m pytest dispar_orchestrate -q` | **2396 passed, 1 skipped**, 516 warnings, 1222.20s |
| `cd dagster && python -m pytest dispar_orchestrate/test_load_memory.py -q -s` | **4 passed** (the memory guard): csv +96 MiB, jsonl +75 MiB, gzip +119 MiB, parquet +68 MiB (bound 250) |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` |
| `python3 -m unittest discover -s ops/lint -p 'test_*.py'` | **33 tests, OK** |

**Not verified / not run:**

- The Rust, TypeScript and compose lines of the `AGENTS.md` verification block were **not run**:
  the change is Python only (`dagster/`), no Rust, no `cargo`, no compose, no `src/`.
- `docker compose config` / a real `up`: not run, no compose file was touched.
- The `README`/docs of the feature page were not touched (no product copy changed).

**Environment.** No Dagster venv existed on this machine and `python3 -m venv` fails here
(`ensurepip` is not available), so the venv was built with `python3 -m venv --without-pip` plus
`get-pip.py`, then `pip install -e . pytest==8.3.4` (the CI's own install line). It lives OUTSIDE
the repo (`/tmp/opencode/dagster-venv`) so the working tree stays clean; `pyiceberg==0.12.0` and
`pyarrow==25.0.1` are the pinned versions the plan names.

**pyiceberg 0.12.0 has everything D1 needs** (checked against the installed package, not from
docs): `pyiceberg.io.pyarrow._dataframe_to_data_files(table_metadata, df, io, write_uuid,
counter)`, `Transaction.update_snapshot(snapshot_properties=…)` → `.fast_append()` /
`.overwrite()`, and `SnapshotProducer.append_data_file` / `.delete_data_file`. Nothing is missing,
so the public fallback the plan mentions was not needed.

**Three deviations from the plan's recipe, each with its evidence — please review these.**

1. **ONE counter for the whole load** (`itertools.count`), passed to every group's
   `_dataframe_to_data_files` call. The plan's recipe calls it once per group without a counter.
   `pyiceberg` names a data file `00000-{task_id}-{write_uuid}.parquet` and the task id comes
   from that counter, which `_dataframe_to_data_files` creates fresh per call: with the plan's
   recipe every group of a load gets the SAME file name and the second group OVERWRITES the
   first group's rows. Measured on pyiceberg 0.12.0 before the change (scratch script): a
   two-group append produced one file path twice, and the table held the LAST group's rows
   twice and the first group's rows not at all. The counter is the parameter of the same
   pinned function, so D1's decision ("one snapshot producer per load, memory still bounded per
   group") holds as decided; the recipe as written does not.
2. **`_check_pyarrow_schema_compatible` is called too** (the check `pyiceberg`'s own
   `Transaction.append` makes before it writes). It is not named in the plan, but the plan's
   recipe drops it, and dropping it is not neutral: the write projects the data file on the
   table's schema, so a column the table does not have would be dropped SILENTLY. The plan's
   own `iceberg_stream` comment ("the write is then refused: 'PyArrow table contains more
   columns'") describes that check. It is pinned in the same place as `_dataframe_to_data_files`,
   with the same version, and `test_iceberg_snapshot.py` guards it.
3. **A new module** `adapters/iceberg_snapshot.py` holds the shared write, and
   `adapters/sink.py` imports it from there (`sink.py` already imports `iceberg_stream`; the
   reverse direction would be a cycle). The plan names the behaviour, not the file.

**One test helper moved with the write.** `test_load_modes_dedup_measurement.py::_Failure`
injected "a failure at the write" into `Transaction.append` / `Transaction.overwrite`; neither is
called any more (the write goes through `Transaction.update_snapshot` → producer), so the
injection point is now `Transaction.update_snapshot` — the call that opens the producer of the
load's one snapshot. The tests assert the same things as before (a failed load changes nothing,
leaves nothing pending, and the next load works); the failure is still injected at the write, not
before it. 13 tests were red on that point before the move and are green after it.

**Tests added / changed.**

- `test_iceberg_stream.py`: the multi-group replace and the multi-group append now assert a
  snapshot-count delta of **1** (`lab.snapshots`), and the zero-row load keeps its "one snapshot"
  assertion (it is the snapshot that carries the schema the load just evolved).
- `test_typed_load.py:322` (the retype path): the snapshot-count assertion is now `== 1`
  (it was `<= 2`, "a delete and an append at most").
- `test_iceberg_snapshot.py` (new, 7 tests, no `dlt`, no network): the loud guard of the two
  pinned private functions and of the two producer methods; an append of two groups leaves one
  snapshot and holds BOTH groups' rows (the counter regression); a replace of two groups leaves
  one OVERWRITE snapshot and drops the old rows; a replace of no rows empties the table in one
  snapshot; an append of no rows commits its one snapshot; the one snapshot carries the snapshot
  properties it is given (D2); a group that does not fit the table is refused and nothing is
  committed.

**D2 read as decided:** the snapshot properties of the load are what they were — the streamed
and the retype paths carry none today (`dlt`'s own commits do not; `sink.SinkConfig`'s note
says so), so the one snapshot carries the empty set, and `history.write_versions` (the path that
does carry `rantai.author`) is untouched.

## Review (planner appends below)
