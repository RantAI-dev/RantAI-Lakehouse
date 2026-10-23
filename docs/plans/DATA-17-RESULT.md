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

The API's growth stays between 0 and 50 MiB for every format and size up to
2 GiB (base 45–209 MiB over the run). The chunked transport keeps the API's
memory flat, as slice 1 designed.

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

- **Memory no longer follows the file**: from 256 MiB to 2 GiB the growth
  stays between about 0.9 and 2.3 GiB for every format (before: up to
  9.5 GiB). CSV 2 GiB went from +9,750 to +1,982 MiB.
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
