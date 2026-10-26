"""The upload job's memory does not grow with the file (`PR #186 review BLOCKER 6`).

`docs/plans/DATA-17-RESULT.md` measured the job growing three to five times the size of the
upload for every format (CSV, 2 GiB: +9.75 GiB). The plan's section 11 ("BLOCKER 6,
investigation") found the cause in the write step (`dlt`'s Iceberg job reads the whole load
into one Arrow table) and in two read-ahead caches (`adapters/iceberg_stream.py`,
`parquet_ingest.open_parquet`). This test is the guard: it loads the same kind of file at two
sizes through the job's real path (`file_ingest.run_file_load` and `sink.load_via_sink`, a
local Iceberg table, no network) and fails when the larger load's growth in memory is more
than `MAX_EXTRA_MIB` above the smaller's.

# How it measures

Each load runs in a fresh Python process (an earlier test's allocator would pollute the
numbers of this one). The process notes its resident memory after the imports, samples it
every 20 ms while the job runs, and reports the maximum minus the base, in MiB. Peak
resident memory is what a container's limit counts.

# The bound, and why

Sizes: 32 and 128 MiB, 96 MiB apart, a load of a few seconds each. Measured on this VM on
2026-10-23 (plan section 11), the difference between the two sizes was:

- unfixed code: CSV +461, JSON Lines +430, gzip +499, Parquet +562 MiB (about 4.5 MiB of memory
  per MiB of file);
- fixed code: CSV +107, JSON Lines +79, gzip +131, Parquet +63 MiB (the fixed part of a load,
  about 350 MiB of `dlt`'s steps and `pyarrow`'s pools, does not depend on the file; what is
  left between the sizes is the allocator's noise).

`MAX_EXTRA_MIB = 250` sits between the worst fixed figure (131) and the best unfixed one (430):
it does not fail on noise, and it fails when the memory follows the file again. Both sides were
checked by running this test against the unfixed code, which fails all four formats.

Run on its own with:
`python -m pytest dispar_orchestrate/test_load_memory.py -q`
"""

from __future__ import annotations

import base64
import gzip
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import pytest

# Runs a real dlt pipeline against a local Iceberg table (registered in pyproject.toml);
# CI runs these in `dagster-loader-tests`, the rest in `dagster-unit-tests`.
pytestmark = pytest.mark.loader

SMALL_MIB = 32
LARGE_MIB = 128
MAX_EXTRA_MIB = 250
FORMATS = ("csv", "jsonl", "gzip", "parquet")
COLUMNS = 10


def _cells(count: int, width: int) -> list[str]:
    text = base64.b64encode(os.urandom(count * width * 3 // 4 + 3)).decode("ascii")
    return [text[i * width : (i + 1) * width] for i in range(count)]


def _blocks(total_bytes: int):
    """Blocks of rows (a list per column) of random text, about `total_bytes` of CSV in all.
    Random text does not compress, so a gzip or Parquet file is as large as it is wide."""
    width, rows_per_block = 13, 20_000
    block_bytes = rows_per_block * COLUMNS * (width + 1)
    for _ in range(max(1, total_bytes // block_bytes)):
        yield [_cells(rows_per_block, width) for _ in range(COLUMNS)]


def make_file(fmt: str, mib: int, directory: Path) -> Path:
    """A file of `fmt` about `mib` MiB, written in blocks so that making it is itself flat."""
    names = [f"c{n}" for n in range(COLUMNS)]
    path = directory / f"upload_{fmt}_{mib}"
    total = mib * 1024 * 1024
    if fmt == "parquet":
        import pyarrow as pa
        import pyarrow.parquet as pq

        schema = pa.schema([(name, pa.string()) for name in names])
        with pq.ParquetWriter(path, schema, compression="snappy") as writer:
            for columns in _blocks(total):
                writer.write_table(pa.table(dict(zip(names, columns)), schema=schema))
        return path
    opener = gzip.open if fmt == "gzip" else open
    with opener(path, "wt", encoding="utf-8", newline="") as out:
        if fmt in ("csv", "gzip"):
            out.write(",".join(names) + "\n")
        for columns in _blocks(total):
            for row in zip(*columns):
                out.write(",".join(row) + "\n" if fmt != "jsonl" else json.dumps(dict(zip(names, row))) + "\n")
    return path


def _resident_mib() -> float:
    with open("/proc/self/status", encoding="ascii") as status:
        for line in status:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024
    raise RuntimeError("no VmRSS: this test needs Linux")


def probe(fmt: str, path: Path, work: Path) -> float:
    """Load `path` as the job does and return the growth of resident memory in MiB. Runs in the
    child process (`python -m dispar_orchestrate.test_load_memory FORMAT FILE WORKDIR`)."""
    for name in (
        "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
    ):
        os.environ.pop(name, None)
    os.environ["DLT_DATA_DIR"] = str(work / "dlt")
    from dlt.destinations import filesystem

    from dispar_orchestrate import file_ingest
    from dispar_orchestrate.adapters import sink as sink_module
    from dispar_orchestrate.adapters.sink import SinkConfig

    sink_module.filesystem = lambda **_kw: filesystem(bucket_url=str(work / "bucket"))
    sink_module._install_catalog_env = lambda _config: None

    class Log:
        def info(self, *_a) -> None: ...
        warning = info

    params = file_ingest.FileIngestParams(
        upload_id="u-memory",
        storage_keys=("uploads/memory",),
        format={"csv": "delimited", "gzip": "delimited", "jsonl": "jsonl", "parquet": "parquet"}[fmt],
        compression="gzip" if fmt == "gzip" else "none",
        max_bytes=10**12,
        bronze_table_name="memory_probe",
        load_mode="replace",
        encoding="utf-8",
        delimiter=",",
        header_row=0,
        columns=(),
        on_bad_value="stop",
    )
    base = _resident_mib()
    peak = base
    stop = threading.Event()

    def sample() -> None:
        nonlocal peak
        while not stop.is_set():
            peak = max(peak, _resident_mib())
            time.sleep(0.02)

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    sink_config = SinkConfig("u", "w", "e", "a", "s", "b", "")
    try:
        summary = file_ingest.run_file_load(
            params,
            log=Log(),
            load_config=lambda: sink_config,
            open_object=lambda _config, _key: open(path, "rb"),
            register=lambda *_a, **_k: 0,
            write_report=lambda *_a, **_k: None,
            record=lambda **_k: None,
        )
    finally:
        stop.set()
        sampler.join()
    assert summary.rows and summary.rows > 0, "the load wrote no rows: the measurement would be of nothing"
    return peak - base


def _growth_in_child(fmt: str, mib: int, directory: Path) -> float:
    path = make_file(fmt, mib, directory)
    work = directory / f"work_{fmt}_{mib}"
    work.mkdir()
    done = subprocess.run(
        [sys.executable, "-m", "dispar_orchestrate.test_load_memory", fmt, str(path), str(work)],
        capture_output=True,
        text=True,
        timeout=900,
        check=False,
    )
    assert done.returncode == 0, done.stderr[-2000:]
    path.unlink()
    return float(done.stdout.strip().splitlines()[-1])


@pytest.mark.parametrize("fmt", FORMATS)
def test_the_load_of_a_four_times_larger_file_does_not_need_more_memory(fmt: str) -> None:
    with tempfile.TemporaryDirectory(prefix="load_memory_") as scratch:
        small = _growth_in_child(fmt, SMALL_MIB, Path(scratch))
        large = _growth_in_child(fmt, LARGE_MIB, Path(scratch))
    print(f"{fmt}: {SMALL_MIB} MiB +{small:.0f} MiB, {LARGE_MIB} MiB +{large:.0f} MiB, difference {large - small:+.0f} MiB")
    assert large - small <= MAX_EXTRA_MIB, (
        f"{fmt}: loading {LARGE_MIB} MiB grew the process by {large:.0f} MiB and {SMALL_MIB} MiB by "
        f"{small:.0f} MiB; the job's memory is growing with the file again (BLOCKER 6)"
    )


if __name__ == "__main__":
    print(probe(sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])))
