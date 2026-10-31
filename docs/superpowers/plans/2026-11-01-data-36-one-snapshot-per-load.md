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

## Review (planner appends below)
