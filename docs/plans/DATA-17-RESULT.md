# `DATA-17` result: memory and time of large uploads

Two measurements: before the streaming fix (run 39903340840) and after it
(run 39916318666, `PR #186 review BLOCKER 6`, commit `e2facb1c`). The second
section is the one the cap is set from.

Measured on the throwaway branch `measure/data-17-cap` (cut from pull
request #186 at `860662e7`, with the session cap, the unpacked-size cap and
the Parquet cell cap raised and `MAX_ROWS` lifted, only so the measurement
can run; never merged), workflow `g9-upload-large`, CI run 39903340840, on
2026-10-23. One run on a GitHub-hosted `ubuntu-latest` runner (16 GB of
memory). Harness: `ops/g9/large_upload.py` at `adaaeece`. Each phase is one
upload through the chunked transport, loaded into Bronze and counted back
through ClickHouse.

Memory is read from the containers' cgroup once a second during the phase:
**base** is the container's memory just before the phase, **growth** is the
maximum sampled during the phase minus the base. `memory.peak` could not be
reset on this runner, so the cgroup peak is "since the container started" and
is not used below.

## The API (the transport)

The API's growth stays between 0 and 50 MiB for every format it measured
(CSV, JSON Lines, gzip and Parquet) and every size up to 2 GiB (base 45–209
MiB over the run). The chunked transport keeps the API's memory flat, as
slice 1 designed.

## The job (Dagster code location)

Growth of the job's memory while one upload loads, and the time from the
first byte sent to the counted table:

| Size | CSV | JSON Lines | CSV, gzip (unpacked size; file sent) | Parquet, 10 files, one load |
| --- | --- | --- | --- | --- |
| 256 MiB | +2,176 MiB, 27.4 s | +1,571 MiB, 27.9 s | +2,267 MiB, 27.6 s (194 MiB) | +1,672 MiB, 18.1 s (216 MiB) |
| 512 MiB | +3,283 MiB, 35.2 s | +2,382 MiB, 44.4 s | +3,197 MiB, 35.0 s (390 MiB) | +2,959 MiB, 25.1 s (503 MiB) |
| 1,024 MiB | +4,574 MiB, 45.7 s | +4,143 MiB, 49.9 s | +5,224 MiB, 50.0 s (780 MiB) | +3,167 MiB, 34.8 s (1,007 MiB) |
| 2,048 MiB | **+9,750 MiB**, 75.3 s | **+6,191 MiB**, 74.0 s | **+8,979 MiB**, 75.4 s (1,557 MiB) | **+8,207 MiB**, 58.1 s (2,013 MiB) |

A single Parquet file of 22, 50, 101 and 201 MiB grew the job by about 0.7,
0.8, 1.1 and 1.5 GiB.

Row counts: CSV about 1.9 million rows at every size (wider rows as the size
grows); JSON Lines 0.95 to 1.9 million; Parquet 1.5 to 14 million in one
load.

## What this shows

- **The job's memory is not flat.** It grows with the bytes of the upload,
  at about three to five times the file's size (CSV 2 GiB: +9.5 GiB). Slice 2
  of the plan was to make the job read the upload as a stream; on this
  measurement it does not hold memory flat for any format. At 2 GiB the job
  alone needs 6–10 GiB more memory, which a typical deployment does not give
  one load.
- **Time is not the limit.** Every phase loaded and counted within about
  76 s, 2 GiB included.
- **The row cap is not the binding limit.** Memory follows bytes, not rows:
  1.9 million CSV rows took 2.1 GiB at 256 MiB and 9.5 GiB at 2 GiB.

## Not measured

- A second upload loading at the same time (one upload at a time only).
- Avro and zip at large sizes (gzip and the three other formats only).
- Any runner other than one 16 GB GitHub-hosted runner; the job's memory on a
  customer's deployment is not known from this run.
- Workbooks (Excel stays at 50 MB, decision 9).
- Where in the job the memory goes (dlt, pyarrow, the Iceberg write): not
  profiled.

## After the streaming fix (run 39916318666, 2026-10-23)

Same branch, harness and runner, with `e2facb1c` cherry-picked (the Iceberg
write in 32 MiB groups in one transaction; Parquet read without
`pre_buffer`). Growth of the job's memory while one upload loads, and time:

| Size | CSV | JSON Lines | CSV, gzip (file sent) | Parquet, 10 files, one load |
| --- | --- | --- | --- | --- |
| 256 MiB | +1,282 MiB, 42.0 s | +1,043 MiB, 37.1 s | +1,169 MiB, 43.1 s (194 MiB) | +926 MiB, 28.5 s (216 MiB) |
| 512 MiB | +1,470 MiB, 53.2 s | +1,251 MiB, 68.2 s | +1,530 MiB, 60.4 s (390 MiB) | +1,140 MiB, 34.8 s (503 MiB) |
| 1,024 MiB | +1,893 MiB, 71.1 s | +1,402 MiB, 76.1 s | +1,285 MiB, 86.3 s (780 MiB) | +1,130 MiB, 64.6 s (1,007 MiB) |
| 2,048 MiB | **+1,982 MiB**, 106.8 s | **+1,628 MiB**, 101.8 s | **+867 MiB**, 118.3 s (1,557 MiB) | **+2,273 MiB**, 97.5 s (2,013 MiB) |

- **Memory no longer follows the file**: for the four formats this run
  measured -- CSV, JSON Lines, a gzipped CSV and ten Parquet files in one load
  -- the growth from 256 MiB to 2 GiB stays between about 0.9 and 2.3 GiB
  (before: up to 9.5 GiB). CSV 2 GiB went from +9,750 to +1,982 MiB.
- **Not measured after the fix**: a JSON array, an Avro file, a zip, and a
  single Parquet file (the before-fix run measured a single Parquet file, at
  22 to 201 MiB, not at 2 GiB). The fix is in the write step, which every
  format goes through, but that is a reading of the code, not a measurement.
- **Parquet still rises a little** (1.1 to 2.3 GiB from 1 to 2 GiB): the
  reader decodes one row group whole, and these files carry larger row groups
  as they grow. A single row group larger than memory is not covered.
- **Time grew by 30 to 60 percent** (CSV 2 GiB: 75 s to 107 s), the cost of
  writing in groups; every load still finishes within two minutes.
- The local measurement on this VM (64 to 1,024 MiB) gave a smaller constant,
  350 to 500 MiB; the CI figure includes the run's own process and is the one
  to plan with.

**Cap for T6.2:** 2 GiB per upload (the spec's target, Databricks' number) is
within these measurements: a 2 GiB load needs about 2.3 GiB more for the job
than an idle one. Not measured, and so not claimed: two such loads at the same
time.

## Several loads at once, and the files not yet covered (`DATA-33`, `DATA-34`, 2026-10-31)

The same harness (`ops/g9/large_upload.py`, with `--concurrent` and the new writers from branch
`measure/data-33-34-large-uploads`), the same workflow (`g9-upload-large`) and the same 16 GB
GitHub-hosted runner. The caps are as on `main`: none was raised. The planner ran the
measurements; the plan is `docs/superpowers/plans/2026-11-01-data-33-34-large-upload-measurements.md`.

### `DATA-33`: N loads of a 2 GiB CSV at once

Each run sends the same 2,048 MiB CSV N times, then starts every load together, each into its own
table. The "growth" column is the Dagster code location's memory during the phase: the max
sampled once a second, minus the base just before.

| Loads at once | Run | Loads that held every row | Job growth | Each load took | Phase took | OOM-killed |
| --- | --- | --- | --- | --- | --- | --- |
| 1 (for comparison, above) | 39916318666 | 1/1 | +1,982 MiB | 106.8 s | 106.8 s | none |
| 2 | 41239217843 | 2/2 | **+6,423 MiB** | 116.4–116.6 s | 136.4 s | none |
| 4 | 41239223375 | 4/4 | **+8,921 MiB** | 215.6–221.5 s | 262.9 s | none |
| 16 | 41239229020 | 4/16 | +7,861 MiB | about 201 s | 386.6 s | none |

- **16 at once was refused by design.** 12 of the 16 loads were refused at once with `429 Too
  many uploads are in progress. Try again in a moment.`, because one user may run 4 loads at
  once (`UPLOAD_MAX_CONCURRENT_PER_USER`, default 4, `SEC-17`). The 4 that ran held every row.
  So the run measured the per-user limit working, not 16 loads.
- **What the measurement shows:**
  - Each load runs in its own Dagster subprocess. Two loads cost +6.3 GiB and four cost
    +8.7 GiB: about **2.2 GiB per 2 GiB load** once several run together.
  - The API stayed flat: growth +112 to +156 MiB.
- **Not measured: 16 loads at once from four users.** The installation cap
  (`UPLOAD_MAX_CONCURRENT`, default 16) allows it, and nothing in Dagster queues runs
  (`dagster/dagster.yaml` sets no run coordinator). By the measured slope, 16 large loads would
  need roughly 35 GiB, which is more than a 16 GB host has. That is an **estimate**, not a
  measurement. It is why the decision below caps large loads.

### `DATA-34`: the files the first runs did not cover

Run 41243053447. A first try (41239234540) generated an Avro file of 2,049 MiB, which the API
correctly refused as over the 2 GB limit, so the run was repeated at 2,000 MiB.

| File | Size | Rows | Job growth | Load took | OOM-killed |
| --- | --- | --- | --- | --- | --- |
| Avro | 2,001 MiB | 1,887,621 | +3,102 MiB | 55.3 s | none |
| Zip of CSVs | 17 MiB sent; 2,000 MiB unpacked | 1,906,501 | +1,135 MiB | 55.5 s | none |
| Parquet, one row group | 1,997 MiB | 1,852,174 | +5,032 MiB | 50.3 s | none |

- **A Parquet file of one 2 GiB row group loads.** The reader decodes the group whole, and it costs
  +4.9 GiB, against +2.2 GiB for ten ordinary files of the same size. It fits on a 16 GB host,
  but two such files at once would not leave much room.
- **The zip's CSVs repeat their cells, so it compresses to 17 MiB.** The unpacked size is what
  the cap counts and what the job reads. A zip of real data would be larger on the wire, with the
  same load.

### Decision (planner, `DATA-33` D5)

- **The limits say it now.** The upload limits name the measured cost (about 2.2 GiB of job
  memory per 2 GiB load, +4.9 GiB for a one-group Parquet), and say that the installation
  allows 16 loads at once, which can run out of memory on a 16 GB host with large files.
- **Follow-up `DATA-37`:** a separate installation-wide limit on **large** loads at once.
  - Proposed: 4 loads of over 512 MiB at once, the level measured as safe on 16 GB.
  - It is configurable, and the next large load is refused with a message, as the other caps do.
  - The product owner confirms the number.
