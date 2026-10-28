#!/usr/bin/env python3
"""Measures the child that reads an uploaded workbook or a Parquet file (`SEC-22` T5).

The child of `SEC-22` runs under a memory limit of its own, and that limit
has to be a number this installation can afford: the peak of the largest
file the upload cap allows, times a factor, with two conversions at once
in mind.

What is measured here, on the machine that runs it:

- `convert-sheet` on a workbook of 5,000,000 cells: the largest sheet the
  cell cap allows (`upload_workbook::MAX_SHEET_CELLS`).
- `preview-parquet` on a Parquet file of 50 MiB whose first row group is
  the whole file: the largest file the upload cap allows
  (`routes::uploads::MAX_UPLOAD_BYTES`), and the row group a preview
  decodes (`upload_parquet::MAX_PREVIEW_GROUP_BYTES`).
- the baseline: the child with an empty input, which is the binary's own
  mappings and its allocator.

The numbers are the peak resident set size and the wall-clock time of
each run, measured with `/usr/bin/time -v`. They are for the handoff; the
planner writes them into `docs/plans/SEC-22-RESULT.md`, and the default in
`config.rs` is set from them.

The Parquet file needs `pyarrow`, which is not a dependency of the repo:
unpack its wheel into a scratch directory and put that directory on
`PYTHONPATH` (the way `ops/fixtures/parquet/make_parquet.py` does). Without
it the Parquet measurement is skipped, and says so.

The workbook is written by the standard library alone: a zip with a sheet
of inline strings. Inline strings are not the shared strings of a
workbook Excel writes, but the reader holds the same text either way, so
the peak is the same peak.

Usage:

    python3 ops/g9/sec22_measure.py <path to the built lakehouse-api> [scratch directory]
"""

from __future__ import annotations

import json
import os
import pathlib
import random
import re
import shutil
import string
import subprocess
import sys
import time
import zipfile

# The cells of the workbook: 100 columns of 50,000 rows is 5,000,000, the
# most a sheet may hold (`upload_workbook::MAX_SHEET_CELLS`).
WORKBOOK_COLUMNS = 100
WORKBOOK_ROWS = 50_000

# The Parquet file: 50 MiB is the most a whole upload may be
# (`routes::uploads::MAX_UPLOAD_BYTES`), and the text is random so that it
# does not compress: the row group a preview decodes is then as big as the
# file allows (`upload_parquet::MAX_PREVIEW_GROUP_BYTES` is 64 MiB).
PARQUET_MIB = 50
PARQUET_ROWS = 500_000
PARQUET_VALUE_CHARS = 100

# The limit the child runs under while it is measured: high enough that the
# limit is not what ends the run, so the peak is the peak of the work.
MEASURE_MEMORY_MIB = 4096
MEASURE_CPU_SECONDS = 3600

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>
"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>
"""

WORKBOOK = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Big" sheetId="1" r:id="rId1"/></sheets>
</workbook>
"""

WORKBOOK_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>
"""

SHEET_HEAD = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
"""

SHEET_TAIL = """</sheetData>
</worksheet>
"""


def column_name(index: int) -> str:
    """The name of the column at `index`, as Excel names it (A, B, ... AA)."""
    name = ""
    number = index
    while True:
        name = chr(ord("A") + number % 26) + name
        number = number // 26 - 1
        if number < 0:
            return name


def row_xml(row: int, columns: int) -> str:
    """The XML of one row of the sheet: `columns` cells, each with a text."""
    cells = []
    for index in range(columns):
        reference = f"{column_name(index)}{row}"
        value = f"row{row}-col{index}"
        cells.append(
            f'<c r="{reference}" t="inlineStr"><is><t>{value}</t></is></c>'
        )
    return f'<row r="{row}">' + "".join(cells) + "</row>"


def write_workbook(path: pathlib.Path, rows: int, columns: int) -> None:
    """Write a workbook of `rows` rows and `columns` columns of text.

    The standard library alone: a zip with the five parts a reader needs.
    The cells are inline strings, so no shared-strings part is written.
    """
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as book:
        book.writestr("[Content_Types].xml", CONTENT_TYPES)
        book.writestr("_rels/.rels", ROOT_RELS)
        book.writestr("xl/workbook.xml", WORKBOOK)
        book.writestr("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
        with book.open("xl/worksheets/sheet1.xml", "w") as sheet:
            sheet.write(SHEET_HEAD.encode())
            for row in range(1, rows + 1):
                sheet.write(row_xml(row, columns).encode())
            sheet.write(SHEET_TAIL.encode())


def write_parquet(path: pathlib.Path) -> int:
    """Write a Parquet file of one row group of random text.

    Needs `pyarrow`; the caller has already checked that it is there. The
    text is random, so the file does not compress: the row group a preview
    decodes is as big as the file is.
    """
    import pyarrow as pa
    import pyarrow.parquet as pq

    alphabet = string.ascii_letters + string.digits
    generator = random.Random(20261028)
    values = [
        "".join(generator.choice(alphabet) for _ in range(PARQUET_VALUE_CHARS))
        for _ in range(PARQUET_ROWS)
    ]
    table = pa.table({"value": pa.array(values, type=pa.string())})
    pq.write_table(
        table,
        path,
        row_group_size=PARQUET_ROWS,
        compression="none",
        use_dictionary=False,
    )
    return path.stat().st_size


def measure(binary: pathlib.Path, operation: list[str], source: pathlib.Path) -> dict:
    """Run the child on `source` under `/usr/bin/time -v`, and read the numbers.

    The child's own limits are set high: the run being measured must not
    end because of them.
    """
    environment = dict(os.environ)
    environment["LAKEHOUSE_CONVERT_MEMORY_MIB"] = str(MEASURE_MEMORY_MIB)
    environment["LAKEHOUSE_CONVERT_CPU_SECONDS"] = str(MEASURE_CPU_SECONDS)
    with source.open("rb") as handle:
        started = time.monotonic()
        finished = subprocess.run(
            ["/usr/bin/time", "-v", str(binary), "inspect-upload", *operation],
            stdin=handle,
            # The child's text is not what is measured, and a workbook of
            # 5,000,000 cells is a hundred megabytes of it.
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )
        elapsed = time.monotonic() - started
    report = finished.stderr.decode("utf-8", "replace")
    peak = re.search(r"Maximum resident set size \(kbytes\): (\d+)", report)
    wall = re.search(r"Elapsed \(wall clock\) time [^:]*: ([\d:.]+)", report)
    return {
        "operation": " ".join(operation),
        "file": source.name,
        "file_bytes": source.stat().st_size,
        "exit_code": finished.returncode,
        "peak_rss_kbytes": int(peak.group(1)) if peak else None,
        "wall_seconds": round(elapsed, 2),
        "time_reported": wall.group(1) if wall else None,
        "stderr_tail": report.strip().splitlines()[-3:],
    }


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    binary = pathlib.Path(argv[1]).resolve()
    if not binary.is_file():
        print(f"[sec22] the binary {binary} is not there")
        return 2
    scratch = pathlib.Path(argv[2]).resolve() if len(argv) > 2 else None
    if scratch is None:
        scratch = pathlib.Path("/tmp/sec22-measure")
    scratch.mkdir(parents=True, exist_ok=True)

    print(f"[sec22] measuring {binary}")
    workbook = scratch / "big.xlsx"
    print(f"[sec22] writing {workbook} ({WORKBOOK_COLUMNS} x {WORKBOOK_ROWS} cells)")
    started = time.monotonic()
    write_workbook(workbook, WORKBOOK_ROWS, WORKBOOK_COLUMNS)
    print(f"[sec22] the workbook is {workbook.stat().st_size} bytes")

    parquet = scratch / "big.parquet"
    parquet_written = False
    try:
        import pyarrow  # noqa: F401

        print(f"[sec22] writing {parquet}")
        write_parquet(parquet)
        parquet_written = True
    except ImportError:
        print(
            "[sec22] pyarrow is not on PYTHONPATH: the Parquet measurement is "
            "skipped (unpack its wheel and put it on PYTHONPATH)"
        )

    empty = scratch / "empty.bin"
    empty.write_bytes(b"")

    results = [
        measure(binary, ["list-sheets"], empty),
        measure(binary, ["convert-sheet"], workbook),
    ]
    if parquet_written:
        results.append(measure(binary, ["preview-parquet", "20"], parquet))

    print(f"[sec22] the fixtures took {round(time.monotonic() - started, 1)} s")
    print(json.dumps(results, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
