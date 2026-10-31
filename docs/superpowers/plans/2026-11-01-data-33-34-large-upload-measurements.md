# DATA-33 and DATA-34: measure concurrent large loads and the large files not yet measured

Backlog `DATA-33` (P2) and `DATA-34` (P3), reassigned to the planner lane on 2026-11-01. Base:
`main` at `9131dced`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code
disagrees, stop and report it.

## 0. Today

- `docs/plans/DATA-17-RESULT.md` measured **one** load at a time, on a 16 GB GitHub runner,
  through the manual workflow `.github/workflows/g9-upload-large.yml` and the harness
  `ops/g9/large_upload.py`. At 2 GiB, the job grew by up to **+9.7 GiB**.
- `SEC-17` lets up to 16 loads run at once per installation. **Nobody knows what two or more 2 GB
  loads at once do.** That is `DATA-33`.
- **Not measured** (`DATA-34`):
  - a Parquet file whose single row group is larger than memory (the reader decodes a row group
    whole);
  - Avro near 2 GB;
  - a zip near 2 GB.

## 1. Decisions

- **D1.** `large_upload.py` gains `--concurrent N` (default 1).
  - N uploads of the same format and size go through the chunked transport first.
  - **Then all N loads start together**, and the harness waits for every one.
  - Memory is sampled once a second from the job's container, as today, and the phase reports:
    - the growth (max minus base);
    - each load's time;
    - each load's result.
  - A load that fails, or the container being OOM-killed (read `docker inspect
    .State.OOMKilled`), **is a result to record, not a harness error**.
- **D2.** New phases for `DATA-34`:
  - `--avro-mb`;
  - `--zip-mb`: one zip holding CSVs that unpack to the size;
  - `--parquet-one-group-mb`: one Parquet file written with a single row group of that size.

  Default each to 0 (skip), so nothing changes for existing runs.
- **D3.** The workflow `g9-upload-large.yml` gains the matching `workflow_dispatch` inputs
  (`concurrent`, `avro_mb`, `zip_mb`, `parquet_one_group_mb`), passed straight through.
- **D4. The planner runs the measurements**, not the developer: the workflow runs on GitHub's
  runner, the same as `DATA-17`, for comparable numbers. The runs are:
  1. CSV, 2048 MiB, with `concurrent` at 2, 4 and 16 (three runs);
  2. one run with avro_mb 2048, zip_mb 2048 and parquet_one_group_mb 2048.

  The caps (session, unpacked) stay **as on `main`**. If a measurement needs a cap raised, it
  runs on this throwaway branch only, never merged, as `DATA-17` did, and the RESULT says so.
- **D5. After the numbers**, the planner decides what follows, and records the decision in the
  RESULT and on the feature page (`upload-file.md` limits):
  - either a lower limit on **large** loads at once (a separate cap for loads over a size), in a
    follow-up item;
  - or a line in the limits.

## 2. Tasks (developer)

- **T1.** D1 and D2 in `ops/g9/large_upload.py`.
  - Unit tests in `ops/g9/test_large_upload.py` (create it if missing) for:
    - the argument parsing;
    - the fixture generators: the Avro file is readable, the zip unpacks to about the size, and
      the Parquet file has exactly one row group.

    Run them at small sizes.
  - A real end-to-end run is not needed here.
  - *Accept:* `python3 -m pytest ops/g9 -q`; `python3 -m py_compile ops/g9/large_upload.py`.
- **T2.** D3. *Accept:* `actionlint .github/workflows/g9-upload-large.yml`, if available.
- **T3.** Docs. Add a "Not yet measured, to run" section to `DATA-17-RESULT.md` naming the D4
  runs. The planner fills in the numbers.

## 3. Handoff (developer appends below)

## 4. Review and results (planner appends below)

### Handoff (written by the planner from the developer's commits)

The developer's session ended on the MiniMax usage limit (error 2056) after it had pushed all
three tasks, so it wrote no handoff. The commits are:
- `ad4cd9de`: T1, the harness (`--concurrent`, the OOM check, the Avro, zip and one-group Parquet
  writers);
- `80be1c24`: T2, the workflow inputs;
- `187a1642`: T3, the RESULT section.

### Review round 1 (planner, 2026-11-01, at `187a1642`)

- **D1.**
  - `--concurrent N` sends N copies, then starts every load together (a thread pool) and waits
    for all of them.
  - `docker inspect .State.OOMKilled` is read, and an OOM is recorded as a result.
- **D2.** `write_avro` (fastavro), `write_csv_zip` (deflated, about the size unpacked) and
  `write_parquet_one_group` (`row_group_size = rows`).
- **D3.** The workflow passes the new inputs through.
- **I ran:** `pytest ops/g9/test_large_upload.py` in a fresh venv (pytest 8.3.4, pyarrow 25.0.1,
  fastavro): **29 passed**.

**Open: no BLOCKER, no SHOULD-FIX.** Next is D4: the planner runs the measurements on this branch.
