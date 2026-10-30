# SEC-22 result: what reading an uploaded workbook or Parquet file costs, and the limits set from it

Date: 2026-10-28 to 2026-10-29. Plan:
`docs/superpowers/plans/2026-10-16-sec-22-upload-conversion-memory.md` (D7, section 6, review
round 1 BLOCKER 5). Written by the planner from the developer's measurements in the plan's
section 7 and the commit bodies of `bbeab4d5` (T1), `af2be706` (T2) and `0fa53a78` (B5). The
planner re-checked the arithmetic but did not re-run the measurement.

## What was measured and what was not

Measured: the child process `lakehouse-api inspect-upload <operation>`, run with `/usr/bin/time
-v` by `ops/g9/sec22_measure.py`. Two builds were run:

- the debug build of the branch, for T1 and T2 only;
- the **release** binary of the gate's image, for the limits: `rantai-lakehouse-api:local`, built
  from the branch with `docker compose build lakehouse-api`, binary `/usr/local/bin/lakehouse-api`,
  run inside that image.

The machine was the development VM (11.9 GB of RAM, Linux 6.8). The fixtures are built at run
time by the script; none of the large ones is committed.

**Not measured (not verified):**

- A preview through the console's proxy (130 s timeout). The release wall times below are far
  under the 90 s wall clock the API puts on the child, which answers first, but the full click
  was not timed.
- Any machine other than this VM. The numbers are this binary on this kernel, not a guarantee
  for another host.
- `RLIMIT_DATA` use itself. Peak RSS was measured. The limit is on the data segment, which also
  counts reserved, untouched memory. The pass/fail runs "at the limits" below cover that gap:
  each file was run under the product's own limits and passed.

## 1. The failure is real (T1, debug build)

Each hostile fixture in `ops/fixtures/uploads/hostile/` was read by today's library code in a
child process under `ulimit -v 524288` (512 MiB):

| fixture | size | what it declares | result |
| --- | --- | --- | --- |
| `huge_shared_strings.xlsx` | 7,983 B | 2,000,000,000 shared strings | the reader process aborted, exit 134: "memory allocation of 48000000000 bytes failed" (`calamine` 0.37 reserves room for every declared string before reading one) |
| `huge_footer.parquet` | 123 B | one row group of 1 TiB | refused with the reader's own sentence; `check_layout` bounds what the footer declares |
| `huge_list_footer.parquet` (review round 1) | 48 B | 2,000,000,000 row groups | the reader process aborted, exit 134: "memory allocation of 192000000000 bytes failed" (`parquet` 58.4.0 reserves `Vec::with_capacity(list size)` during the footer decode, before `check_layout` runs) |

In each case the process that ran the fixture was still alive. Inside the API, the same read
ends the API for every tenant, because Rust aborts on an allocation failure and `catch_unwind`
cannot catch an abort.

The `.xls` reader does not read the shared-string table's count (`calamine` 0.37, `src/xls.rs`,
`parse_sst`), so a false count there costs nothing.

## 2. Which limit (T2, debug build, the same VM)

| limit set by the child on itself | valid workbook | `huge_shared_strings.xlsx` |
| --- | --- | --- |
| `RLIMIT_AS` = 64 MiB | dies of SIGSEGV (exit 139) before reading | dies the same way |
| `RLIMIT_DATA` = 64 MiB | read | aborts (exit 134), "memory allocation of 48000000000 bytes failed" |

`RLIMIT_AS` counts the binary's mappings, the stacks and the allocator's arenas, so it kills a
child that has done nothing yet. The child therefore sets `RLIMIT_DATA` and `RLIMIT_CPU`.

## 3. What the largest allowed file costs (release build)

The workbook is 5,000,000 cells (100 columns × 50,000 rows), the most a sheet may hold
(`upload_workbook::MAX_SHEET_CELLS`). "Shared strings" is the same workbook with its text in a
shared-strings part, which is how Excel writes one. The Parquet file's first row group is
61 MiB, near `MAX_PREVIEW_GROUP_BYTES` (64 MiB). The preview reads the route's 2,000 rows, fed by
ranges (plan P1: the file's tail plus that row group).

| run | peak RSS | CPU (user + sys) | wall | exit | exit at the product's limits |
| --- | --- | --- | --- | --- | --- |
| baseline, empty input | 9.9 MiB | 0.00 s | 0.00 s | 2 (refused: nothing to read) | 2 |
| `convert-sheet`, inline strings | 523.8 MiB | 4.94 s | 5.05 s | 0 | 0 |
| `convert-sheet`, shared strings | 801.5 MiB | 6.10 s | 6.25 s | 0 | 0 |
| `preview-parquet`, 2,000 rows | 163.5 MiB | 0.18 s | 0.21 s | 0 | 0 |

"The product's limits" are 1,280 MiB of `RLIMIT_DATA` and 60 s of `RLIMIT_CPU`.

Under the first default, 896 MiB, which came from a debug-build measurement of the inline-strings
workbook (T5, `546ed291`), the shared-strings workbook **aborted** (exit 134). That default
refused the largest allowed file in the form Excel actually writes. It is superseded.

The debug build took 82.9 s of wall time for the inline-strings sheet. It is not the deployed
binary, so it says nothing about the limits.

## 4. The limits

| limit | value | from |
| --- | --- | --- |
| child memory (`RLIMIT_DATA`), `upload_convert::DEFAULT_MEMORY_MIB`, override `UPLOAD_CONVERT_MEMORY_MB` | **1,280 MiB** | 801.5 MiB peak × 1.5 = 1,202.3 MiB, rounded up to the next 128 MiB |
| child CPU (`RLIMIT_CPU`) | 60 s (the plan's number, unchanged) | the largest file takes 6.10 s; × 1.5 = 9.2 s, under the limit |
| wall clock on the child, set by the API | 90 s (the plan's number, unchanged) | the largest file takes 6.25 s; × 1.5 = 9.4 s, under it; the console's proxy gives up at 130 s |
| conversions at once per API process, `UPLOAD_MAX_CONVERSIONS` | 2 (the plan's number) | 2 × 1,280 MiB = 2.5 GiB at most, which a 12 GB machine affords next to the stack |

All four are caps a measurement admits, not targets. A larger sheet is refused before the child
runs (`MAX_SHEET_CELLS`), and the child refuses more than 512 MiB of converted text
(`MAX_OUTPUT_BYTES`).

An operator who lowers `UPLOAD_CONVERT_MEMORY_MB` below about 1,200 MiB will see the largest
shared-strings workbooks refused with "This file needs more memory or time to read than an
upload is allowed". That is the intended failure mode, and the API keeps running.
