#!/usr/bin/env python3
"""Measures the child that reads an uploaded workbook or a Parquet file (`SEC-22` T5, review BLOCKER 5).

The child of `SEC-22` runs under a memory limit of its own, and that limit
has to be a number this installation can afford: the peak of the largest
file the upload cap allows, times a factor, with two conversions at once
in mind.

What is measured here, on the machine that runs it:

- `convert-sheet` on a workbook of 5,000,000 cells whose text is INLINE:
  the largest sheet the cell cap allows
  (`upload_workbook::MAX_SHEET_CELLS`).
- `convert-sheet` on the same workbook with the text in a SHARED-STRINGS
  part: the way Excel writes a workbook. The reader reserves room for the
  whole table before it reads a cell, so the two numbers are not the same.
- `preview-parquet` on a Parquet file whose first row group is near
  `upload_parquet::MAX_PREVIEW_GROUP_BYTES`, with the preview's 2,000 rows:
  the group a preview decodes (`SEC-22` P1: the child is given the file's
  tail and that group, and never the whole file).
- the baseline: the child with an empty input, which is the binary's own
  mappings and its allocator.

The numbers are the peak resident set size, the user and system CPU time
and the wall-clock time of each run, measured with `/usr/bin/time -v`. They
are for the handoff; the planner writes them into
`docs/plans/SEC-22-RESULT.md`, and the default in `upload_convert.rs` is
set from them.

Every run must ANSWER, and the Parquet preview must read the rows it was
asked for: a run that read the wrong range still exits 0, so the script
fails when the answer is missing or the rows are not there
(`PR #187` review round 2 S5).

Every fixture is run TWICE: once with the child's limits out of the way (so
the peak is the peak of the work and not of the limit), and once with the
LIMITS AS THEY ARE, which is the pass or fail of the file under them. The
exit code of the second run is what the handoff quotes.

The child runs where the deployment runs it: inside the image the gate
builds (`--image`), with the binary at `/usr/local/bin/lakehouse-api`. The
host's `/usr/bin/time` is mounted into the container: it needs nothing but
the C library, and the container's is newer than the host's, so it runs
there. Without `--image` the binary is run on the host, which is what a
developer with a release build at hand does.

The Parquet file needs `pyarrow`, which is not a dependency of the repo:
unpack its wheel into a scratch directory and put that directory on
`PYTHONPATH` (the way `ops/fixtures/parquet/make_parquet.py` does). Without
it the Parquet measurement is skipped, and says so.

The workbooks are written by the standard library alone: a zip with a sheet
of text.

Usage:

    python3 ops/g9/sec22_measure.py <path to the built lakehouse-api> [scratch directory] [--image <image>]
"""

from __future__ import annotations

import json
import os
import pathlib
import random
import re
import string
import subprocess
import sys
import time
import zipfile

# The cells of the workbook: 100 columns of 50,000 rows is 5,000,000, the
# most a sheet may hold (`upload_workbook::MAX_SHEET_CELLS`).
WORKBOOK_COLUMNS = 100
WORKBOOK_ROWS = 50_000

# The Parquet file: its first row group is near
# `upload_parquet::MAX_PREVIEW_GROUP_BYTES` (64 MiB), which is the most a
# preview fetches, and the preview decodes 2,000 rows
# (`routes::uploads::PARQUET_PREVIEW_ROWS`).
PARQUET_GROUP_BYTES = 60 * 1024 * 1024
PARQUET_ROWS = 300_000
PARQUET_VALUE_CHARS = 200
PARQUET_PREVIEW_ROWS = 2000

# The bytes of a Parquet file's end the child is given: the footer and the
# eight bytes that follow it (`upload_convert::PARQUET_TAIL_BYTES`).
PARQUET_TAIL_BYTES = 16 * 1024 * 1024 + 8

# The limits the child runs under while it is MEASURED: high enough that the
# limit is not what ends the run, so the peak is the peak of the work.
MEASURE_MEMORY_MIB = 4096
MEASURE_CPU_SECONDS = 3600

# The limits the child runs under while it is CHECKED: the ones the product
# uses (`upload_convert::DEFAULT_MEMORY_MIB` and `DEFAULT_CPU_SECONDS`).
CHECK_MEMORY_MIB = 1280
CHECK_CPU_SECONDS = 60

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>
"""

CONTENT_TYPES_SHARED = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
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

SHARED_HEAD = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{count}" uniqueCount="{count}">
"""

SHARED_TAIL = """</sst>
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


def cell_text(row: int, column: int) -> str:
    """The text of one cell."""
    return f"row{row}-col{column}"


def row_xml(row: int, columns: int) -> str:
    """The XML of one row of the sheet: `columns` cells, each with a text."""
    cells = []
    for index in range(columns):
        reference = f"{column_name(index)}{row}"
        value = cell_text(row, index)
        cells.append(
            f'<c r="{reference}" t="inlineStr"><is><t>{value}</t></is></c>'
        )
    return f'<row r="{row}">' + "".join(cells) + "</row>"


def row_xml_shared(row: int, columns: int) -> str:
    """The XML of one row of the sheet: `columns` cells, each with the index
    of its text in the shared-strings part."""
    cells = []
    for index in range(columns):
        reference = f"{column_name(index)}{row}"
        value = (row - 1) * columns + index
        cells.append(f'<c r="{reference}" t="s"><v>{value}</v></c>')
    return f'<row r="{row}">' + "".join(cells) + "</row>"


def write_workbook(path: pathlib.Path, rows: int, columns: int) -> None:
    """Write a workbook of `rows` rows of `columns` cells of text.

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


def write_shared_workbook(path: pathlib.Path, rows: int, columns: int) -> None:
    """Write a workbook of `rows` rows of `columns` cells of text, the text
    in a shared-strings part.

    This is the way Excel writes a workbook: every cell holds the index of
    its text in one shared table, and the reader reserves room for the whole
    table before it reads a cell (`calamine`). The inline-strings workbook
    does not, so the two together say what a real workbook costs.
    """
    cells = rows * columns
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as book:
        book.writestr("[Content_Types].xml", CONTENT_TYPES_SHARED)
        book.writestr("_rels/.rels", ROOT_RELS)
        book.writestr("xl/workbook.xml", WORKBOOK)
        book.writestr("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
        with book.open("xl/sharedStrings.xml", "w") as strings:
            strings.write(SHARED_HEAD.format(count=cells).encode())
            for row in range(1, rows + 1):
                for index in range(columns):
                    strings.write(f"<si><t>{cell_text(row, index)}</t></si>".encode())
            strings.write(SHARED_TAIL.encode())
        with book.open("xl/worksheets/sheet1.xml", "w") as sheet:
            sheet.write(SHEET_HEAD.encode())
            for row in range(1, rows + 1):
                sheet.write(row_xml_shared(row, columns).encode())
            sheet.write(SHEET_TAIL.encode())


def write_parquet(path: pathlib.Path) -> int:
    """Write a Parquet file whose first row group is near the preview bound.

    Needs `pyarrow`; the caller has already checked that it is there. The
    text is random, so the file does not compress: the row group a preview
    decodes is as big as it is asked to be.
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


def parquet_input(path: pathlib.Path, size: int) -> tuple[list[str], pathlib.Path]:
    """The arguments and the input of a Parquet preview (`SEC-22` P1).

    The child is given the file's TAIL (the footer is in it) and the first
    row group's bytes, and it is told the file's size and where the group
    is. The prepared input is written beside the file.
    """
    import pyarrow.parquet as pq

    metadata = pq.ParquetFile(path).metadata
    group = metadata.row_group(0)
    columns = [group.column(index) for index in range(group.num_columns)]

    def start_of(column: object) -> int:
        """Where a column chunk starts: its dictionary page when it has one,
        its data page otherwise (the same rule the crate's `byte_range`
        uses)."""
        if getattr(column, "has_dictionary_page", False) and (
            getattr(column, "dictionary_page_offset", None) is not None
        ):
            return column.dictionary_page_offset
        return column.file_offset

    start = min(start_of(column) for column in columns)
    end = max(
        start_of(column) + column.total_compressed_size for column in columns
    )
    tail_len = min(size, PARQUET_TAIL_BYTES)
    with path.open("rb") as handle:
        handle.seek(size - tail_len)
        tail = handle.read(tail_len)
    with path.open("rb") as handle:
        handle.seek(start)
        group_bytes = handle.read(end - start)
    prepared = path.parent / f"{path.name}.preview-input"
    prepared.write_bytes(tail + group_bytes)
    return (
        [
            "preview-parquet",
            str(PARQUET_PREVIEW_ROWS),
            str(size),
            str(start),
            str(end - start),
        ],
        prepared,
    )


def command_for(
    binary: pathlib.Path,
    operation: list[str],
    image: str | None,
    scratch: pathlib.Path,
    memory_mib: int,
    cpu_seconds: int,
) -> list[str]:
    """The command that runs the child, with `/usr/bin/time -v` in front."""
    if image is None:
        return ["/usr/bin/time", "-v", str(binary), "inspect-upload", *operation]
    return [
        "docker",
        "run",
        "--rm",
        "-i",
        # The image's own entrypoint starts the API; the measurement runs the
        # child in it, with `/usr/bin/time` in front.
        "--entrypoint",
        "/usr/bin/time",
        # The child's limits are set INSIDE the container: `docker run` does
        # not pass the host's environment into it.
        "-e",
        f"LAKEHOUSE_CONVERT_MEMORY_MIB={memory_mib}",
        "-e",
        f"LAKEHOUSE_CONVERT_CPU_SECONDS={cpu_seconds}",
        "-v",
        "/usr/bin/time:/usr/bin/time:ro",
        "-v",
        f"{scratch}:/data:ro",
        image,
        "-v",
        "/usr/local/bin/lakehouse-api",
        "inspect-upload",
        *operation,
    ]


def measure(
    operation: list[str],
    source: pathlib.Path,
    binary: pathlib.Path,
    image: str | None,
    scratch: pathlib.Path,
    memory_mib: int,
    cpu_seconds: int,
) -> dict:
    """Run the child on `source` under `/usr/bin/time -v`, and read the numbers.

    The child's own limits are the ones named: high enough that the limit is
    not what ends the run, so the peak is the peak of the work.
    """
    environment = dict(os.environ)
    environment["LAKEHOUSE_CONVERT_MEMORY_MIB"] = str(memory_mib)
    environment["LAKEHOUSE_CONVERT_CPU_SECONDS"] = str(cpu_seconds)
    command = command_for(binary, operation, image, scratch, memory_mib, cpu_seconds)
    if image is not None:
        with source.open("rb") as handle:
            finished = subprocess.run(
                command,
                stdin=handle,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=environment,
                check=False,
            )
    else:
        with source.open("rb") as handle:
            started = time.monotonic()
            finished = subprocess.run(
                command,
                stdin=handle,
                # The child's text is read, not thrown away: a measurement
                # that never looks at the answer cannot tell a run that read
                # the file from one that read nothing.
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=environment,
                check=False,
            )
            del started
    report = finished.stderr.decode("utf-8", "replace")
    peak = re.search(r"Maximum resident set size \(kbytes\): (\d+)", report)
    user = re.search(r"User time \(seconds\): ([\d.]+)", report)
    system = re.search(r"System time \(seconds\): ([\d.]+)", report)
    wall = re.search(r"Elapsed \(wall clock\) time.*?: ([\d:.]+)", report)
    answer, rows = answer_of(finished.stdout)
    return {
        "operation": " ".join(operation),
        "file": source.name,
        "file_bytes": source.stat().st_size,
        "exit_code": finished.returncode,
        "peak_rss_kbytes": int(peak.group(1)) if peak else None,
        "user_seconds": float(user.group(1)) if user else None,
        "system_seconds": float(system.group(1)) if system else None,
        "wall_seconds": wall.group(1) if wall else None,
        "stderr_tail": report.strip().splitlines()[-3:],
        "answer": answer,
        "rows": rows,
    }


def answer_of(output: bytes) -> tuple[dict | None, int]:
    """The child's answer line, and the rows of the text after it.

    The answer is one JSON line, and the text after it is the child's rows:
    the first line of it is the header. A run that answered nothing has
    neither.
    """
    newline = output.find(b"\n")
    if newline < 0:
        return None, 0
    try:
        answer = json.loads(output[:newline].decode("utf-8", "replace"))
    except json.JSONDecodeError:
        return None, 0
    rows = max(len(output[newline + 1 :].splitlines()) - 1, 0)
    return answer, rows


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    binary = pathlib.Path(argv[1]).resolve()
    image = None
    scratch = None
    rest = argv[2:]
    if "--image" in rest:
        at = rest.index("--image")
        image = rest[at + 1]
        rest = rest[:at] + rest[at + 2 :]
    if rest:
        scratch = pathlib.Path(rest[0]).resolve()
    if scratch is None:
        scratch = pathlib.Path("/tmp/sec22-measure")
    if image is None and not binary.is_file():
        print(f"[sec22] the binary {binary} is not there")
        return 2
    scratch.mkdir(parents=True, exist_ok=True)

    print(f"[sec22] measuring {binary}" + (f" in {image}" if image else ""))
    started = time.monotonic()

    print(f"[sec22] writing the inline-strings workbook")
    inline = scratch / "big-inline.xlsx"
    write_workbook(inline, WORKBOOK_ROWS, WORKBOOK_COLUMNS)
    print(f"[sec22] the inline-strings workbook is {inline.stat().st_size} bytes")

    print(f"[sec22] writing the shared-strings workbook")
    shared = scratch / "big-shared.xlsx"
    write_shared_workbook(shared, WORKBOOK_ROWS, WORKBOOK_COLUMNS)
    print(f"[sec22] the shared-strings workbook is {shared.stat().st_size} bytes")

    parquet = scratch / "big.parquet"
    parquet_prepared = None
    try:
        import pyarrow  # noqa: F401

        print(f"[sec22] writing the Parquet file")
        write_parquet(parquet)
        operation, parquet_prepared = parquet_input(parquet, parquet.stat().st_size)
        print(
            f"[sec22] the Parquet file is {parquet.stat().st_size} bytes; "
            f"the preview asks for {operation}"
        )
    except ImportError:
        print(
            "[sec22] pyarrow is not on PYTHONPATH: the Parquet measurement is "
            "skipped (unpack its wheel and put it on PYTHONPATH)"
        )

    empty = scratch / "empty.bin"
    empty.write_bytes(b"")

    fixtures = [
        (["list-sheets"], empty, "the baseline"),
        (["convert-sheet"], inline, "convert-sheet, inline strings"),
        (["convert-sheet"], shared, "convert-sheet, shared strings"),
    ]
    if parquet_prepared is not None:
        fixtures.append((operation, parquet_prepared, "preview-parquet, 2000 rows"))

    results = []
    for operation, source, what in fixtures:
        print(f"[sec22] measuring {what} ({source.stat().st_size} bytes)")
        measured = measure(
            operation,
            source,
            binary,
            image,
            scratch,
            MEASURE_MEMORY_MIB,
            MEASURE_CPU_SECONDS,
        )
        measured["what"] = what
        print(f"[sec22] checking {what} under the product's own limits")
        checked = measure(
            operation,
            source,
            binary,
            image,
            scratch,
            CHECK_MEMORY_MIB,
            CHECK_CPU_SECONDS,
        )
        measured["checked_exit_code"] = checked["exit_code"]
        measured["checked_stderr_tail"] = checked["stderr_tail"]
        # The child must have ANSWERED, and the preview must have read the
        # rows it was asked for: a run that read the wrong range still exits
        # 0, and a measurement that does not look at the answer cannot tell
        # the two apart (`PR #187` review round 2 S5).
        if measured["exit_code"] == 0:
            if measured["answer"] is None:
                print(f"[sec22] {what}: the child answered nothing")
                return 1
            if operation[0] == "preview-parquet":
                rows = measured["rows"]
                print(f"[sec22] {what}: the preview read {rows} rows")
                if rows != PARQUET_PREVIEW_ROWS:
                    print(
                        f"[sec22] {what}: the preview did not read "
                        f"{PARQUET_PREVIEW_ROWS} rows"
                    )
                    return 1
        results.append(measured)

    print(f"[sec22] the fixtures took {round(time.monotonic() - started, 1)} s")
    print(json.dumps(results, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
