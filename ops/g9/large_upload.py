#!/usr/bin/env python3
"""Large-file upload harness (`DATA-17` T2.3; plan section 7, slice 2): sends big
files through the chunked routes, loads them, and prints one table of what it cost.

Not an acceptance gate: it asserts that every load succeeded and that the table
holds every row, and otherwise MEASURES. `docs/plans/DATA-17-RESULT.md` (slice 6)
is where its numbers are read; nothing here decides a limit.

# What it does

1. Signs in and joins the gate tenant, as `ops/g9/upload_test.py` does (the gates
   share no code, so the few lines are copied).
2. Generates its files at run time, on disk, in a scratch directory (no file is
   committed): a CSV of `--csv-mb` MiB (2048 by default), `--parquet-files`
   Parquet files of `--parquet-mb` MiB in all (1945 MiB in 10 files by default), a
   JSON Lines file of `--jsonl-mb` MiB (2048) and a gzipped CSV that unpacks to
   `--gzip-mb` MiB (2048; the file sent is smaller, the unpacked size is what the
   cap counts, feature page decision 2). Slice 6 (T6.1) names these four.
   `--sizes 256,512,1024,2048` (MiB) measures each enabled format at each of those
   sizes instead, every one its own phase with its own `RESULT` line (the Parquet
   size is the total over `--parquet-files` files).
   The rows are sized to stay under 2,000,000, as they were when the job had a
   row cap (`file_ingest.MAX_ROWS`, removed in T6.2 item 4), so the figures of
   `docs/plans/DATA-17-RESULT.md` stay comparable run to run: wide rows of random
   text, which also compress poorly, so the Parquet files reach the size asked for.
   Three more formats (`DATA-34`, each 0 = skipped, so a run without them is the run
   of today): an Avro container of `--avro-mb` MiB (the null codec, string fields
   only: the API refuses a compressed container and a binary column), one zip of
   CSVs that unpack to `--zip-mb` MiB (the entries hold repeated text, so the
   archive on disk is far below the session cap while the unpacked size, which is
   what the cap counts, is what was asked for), and one Parquet file of
   `--parquet-one-group-mb` MiB that holds exactly one row group (Parquet writes a
   four-byte length in front of every value, so the row group decodes to a little
   less than the file; the reader decodes a row group whole, so this is the file a
   load cannot read without holding it in memory).
3. Sends each file in 8 MiB pieces (`POST /api/uploads/sessions`, `PUT .../parts/{n}`,
   `POST .../complete`), 4 pieces at a time, and loads it with `replace` into its
   own raw table (`POST .../ingest`), then polls the upload to `ingested`.
   `--concurrent N` (1 by default) sends N copies of the file through that
   transport first and then starts all N loads together, each into its own table,
   and waits for every one (`DATA-33`: `SEC-17` lets several loads run at once,
   and nobody knows what that costs).
4. Counts the rows through the query route (`WHERE 1`, R11) and compares.
   A load that fails, or a container that was OOM-killed (`docker inspect
   .State.OOMKilled`), is a result to record and not a harness error: the phase
   goes on, the other loads are waited for, and the run ends with a non-zero
   status.
   The API refuses a load above the limits of `SEC-17`
   (`UPLOAD_MAX_CONCURRENT_PER_USER`, four by default, and
   `UPLOAD_MAX_CONCURRENT`, sixteen) and above the limit on large loads
   (`DATA-37`: `UPLOAD_MAX_CONCURRENT_LARGE`, two by default): a refused load
   is recorded as a result and not retried, so a run with more loads than the
   limit allows says so instead of loading them one after another.
   The files of this harness are large -- the CSV and the JSON Lines file are
   2,048 MiB by default, over the 512 MiB of the threshold -- so a phase of
   those with `--concurrent 3` or more has its third and later loads refused
   with the large-load sentence ("This installation already runs the maximum
   number of large loads."). That is the expected answer of the limit: the
   refusal is recorded as a result, the run ends with a non-zero status, and
   the limit is NOT raised to make the run pass. A phase whose file is under
   the threshold (a Parquet file of 194 MiB by default) is not large and the
   large limit does not refuse it.
5. While it does all that, samples the memory of the containers named by
   `--containers` (`NAME=container` pairs, e.g. `api=g9large-lakehouse-api-1`,
   `dagster=g9large-dagster-code-location-1`) once a second, by
   `docker exec <container> cat /sys/fs/cgroup/memory.current`, and reads
   `memory.peak` (cgroup v2) at the end of each phase. Every phase is its own
   window (g9-upload-large run 39891862222 printed one identical maximum for three
   phases, because the sampler's maximum and `memory.peak`, which counts since the
   container started and never goes down, carried over): at the start of a phase the
   sampler drops its maximum, records the container's memory then (the baseline) and
   tries to reset `memory.peak` by writing to it (cgroup v2, kernel 6.12 and later,
   and only where the container's cgroup file is writable). Where the write is
   refused the peak stays "since start" and the line says so; the sampled maximum,
   with the baseline beside it, is the per-phase figure either way.
   The Dagster run is a process of the code-location container, so that
   container's figures are the run's. The console (Next.js) is not part of this
   compose project: its proxy was measured on its own (section 8 item 3 of the plan).

# Output

Each phase prints a `[large] RESULT` line the moment it ends (run 39817023398 failed in
its third phase and an end-of-run table is never reached then), then one table, one row per phase: what, bytes, rows, seconds, MiB/s, the loads of the phase (how many of them held every row), then for each
container the sampled maximum and the cgroup peak in MiB, and the free disk of the
scratch directory at the end. A figure that could not be read is `n/a`; nothing
is filled in. The `RESULT` line of a phase with more than one load names every
load: its time and how it ended.

# Settings

`LAKEHOUSE_API_URL` (default `http://localhost:8080`: it runs on the host, not in
the compose network, because it reads the containers' cgroups), `AUTH_BOOTSTRAP_EMAIL`,
`AUTH_BOOTSTRAP_PASSWORD`, `ICEBERG_QUERY_DB` (default `icecat_api`), `G9_TENANT_ID`
(optional). Needs `requests` and `pyarrow`; the Avro file needs `fastavro`.
"""
from __future__ import annotations

import argparse
import base64
import concurrent.futures
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from array import array
from dataclasses import dataclass, field
from pathlib import Path

PART_SIZE = 8 * 1024 * 1024
PIECES_IN_FLIGHT = 4
MIB = 1024 * 1024
# The job's former row cap (`file_ingest.MAX_ROWS`, removed in T6.2 item 4); a generated
# file still stays under it so that a run is comparable with the measured ones.
JOB_MAX_ROWS = 2_000_000
TARGET_ROWS = 1_900_000
# The most files a zip may hold (feature page decision 13; the API reads ten).
ZIP_FILES = 10
# The room an Avro container needs for its header and its sync markers.
AVRO_ROOM_BYTES = 4096
# The room a Parquet file needs for its footer (the metadata of its row groups).
PARQUET_FOOTER_ROOM = 4 * 1024 * 1024

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://localhost:8080")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "")
QUERY_DB = os.environ.get("ICEBERG_QUERY_DB", "icecat_api")
TENANT_ID = os.environ.get("G9_TENANT_ID", "")
GATE_TENANT_SLUG = "meridian-group"
LOAD_TIMEOUT_S = 3 * 3600
VISIBLE_TIMEOUT_S = 120


class HarnessFailure(Exception):
    pass


# ── Files ────────────────────────────────────────────────────────────────


def _csv_header(columns: int) -> str:
    return ",".join(f"c{n}" for n in range(columns)) + "\n"


def row_width(total_bytes: int, columns: int = 10) -> tuple[int, int]:
    """(characters per cell, rows) so that the header and `rows` rows of `columns` cells
    of that width, plus separators, make at most `total_bytes` (never over: the API
    refuses a session above its cap, g9-upload-large run 39803084936) in at most
    `TARGET_ROWS` rows."""
    per_row = max(columns * 12, total_bytes // TARGET_ROWS + 1)
    cell = max(8, (per_row - columns) // columns)
    row_bytes = columns * cell + columns  # cells and separators/newline
    return cell, max(1, (total_bytes - len(_csv_header(columns))) // row_bytes)


def _random_cells(count: int, width: int) -> list[str]:
    """`count` random base64 strings of `width` characters."""
    raw = os.urandom(count * width)
    text = base64.b64encode(raw).decode("ascii")
    return [text[i * width : (i + 1) * width] for i in range(count)]


def write_csv(path: Path, total_bytes: int, columns: int = 10) -> int:
    """A CSV of at most `total_bytes` bytes, written in blocks. Returns its rows."""
    cell, rows = row_width(total_bytes, columns)
    header = _csv_header(columns)
    written = 0
    with path.open("w", encoding="utf-8", newline="") as out:
        out.write(header)
        block = 20_000
        while written < rows:
            n = min(block, rows - written)
            cells = [_random_cells(n, cell) for _ in range(columns)]
            out.write("".join(",".join(c[i] for c in cells) + "\n" for i in range(n)))
            written += n
    return rows


def jsonl_width(total_bytes: int, columns: int = 10) -> tuple[int, int]:
    """(characters per value, rows) so that lines of `columns` keys `"cN":"..."` make
    about `total_bytes` in at most `TARGET_ROWS` rows. Base64 text needs no escaping,
    so a value is exactly `cell` characters."""
    overhead = columns * len('"c0":"",') + 2  # keys, quotes, colons, commas, braces, newline
    per_row = max(columns * 20 + overhead, total_bytes // TARGET_ROWS + 1)
    cell = max(8, (per_row - overhead) // columns)
    return cell, max(1, total_bytes // (columns * cell + overhead))  # exact per line: never over


def write_jsonl(path: Path, total_bytes: int, columns: int = 10) -> int:
    """A JSON Lines file (one object per line) of about `total_bytes` bytes. Returns its rows."""
    cell, rows = jsonl_width(total_bytes, columns)
    keys = [f'"c{n}":"' for n in range(columns)]
    written = 0
    with path.open("w", encoding="utf-8", newline="") as out:
        block = 20_000
        while written < rows:
            n = min(block, rows - written)
            cells = [_random_cells(n, cell) for _ in range(columns)]
            out.write(
                "".join("{" + ",".join(f'{keys[k]}{cells[k][i]}"' for k in range(columns)) + "}\n" for i in range(n))
            )
            written += n
    return rows


def write_csv_gz(path: Path, unpacked_bytes: int, columns: int = 10) -> int:
    """A gzipped CSV that unpacks to at most `unpacked_bytes` bytes, streamed through
    gzip so the plain file is never on disk. Returns its rows."""
    import gzip

    cell, rows = row_width(unpacked_bytes, columns)
    header = _csv_header(columns)
    written = 0
    with gzip.open(path, "wt", encoding="utf-8", newline="", compresslevel=6) as out:
        out.write(header)
        block = 20_000
        while written < rows:
            n = min(block, rows - written)
            cells = [_random_cells(n, cell) for _ in range(columns)]
            out.write("".join(",".join(c[i] for c in cells) + "\n" for i in range(n)))
            written += n
    return rows


def write_parquet(path: Path, total_bytes: int, columns: int = 10) -> int:
    """A Parquet file of at most `total_bytes` bytes on disk (it stops before the row
    group that would cross it, leaving room for the footer). Returns its rows."""
    import pyarrow as pa
    import pyarrow.parquet as pq

    cell, _ = row_width(total_bytes, columns)
    schema = pa.schema([(f"c{n}", pa.string()) for n in range(columns)])
    per_row = columns * cell
    # A group is at most a quarter of the target, so a small target still gets rows.
    batch = max(1, min(50_000, total_bytes // 4 // per_row))
    footer_room = 64 * 1024
    rows = 0
    biggest = batch * per_row  # the raw size bounds a group until one has been measured
    with path.open("wb") as sink, pq.ParquetWriter(sink, schema, compression="snappy") as writer:
        while sink.tell() + biggest + footer_room <= total_bytes and rows + batch <= JOB_MAX_ROWS - 50_000:
            before = sink.tell()
            writer.write_table(pa.table({f"c{k}": _random_cells(batch, cell) for k in range(columns)}, schema=schema))
            biggest = max(sink.tell() - before, 1)
            rows += batch
    return rows


def _avro_length_bytes(length: int) -> int:
    """The bytes the length prefix of an Avro string of `length` characters takes: a
    zig-zag varint."""
    value = (length << 1) ^ (length >> 63)
    size = 1
    while value >= 0x80:
        value >>= 7
        size += 1
    return size


def avro_width(total_bytes: int, columns: int = 10) -> tuple[int, int]:
    """(characters per cell, records) so that `records` records of `columns` strings of
    that width take at most `total_bytes`, the length prefixes included."""
    per_row = max(columns * 12, total_bytes // TARGET_ROWS + 1)
    cell = max(8, (per_row - columns) // columns)
    row_bytes = columns * (cell + _avro_length_bytes(cell))
    # The container writes a header of its own and a 16-byte sync marker per block: a
    # byte per record and 4 KiB of room cover both.
    return cell, max(1, (total_bytes - AVRO_ROOM_BYTES) // (row_bytes + 1))


def write_avro(path: Path, total_bytes: int, columns: int = 10) -> int:
    """An Avro object container file of at most `total_bytes` bytes: the null codec and
    string fields only, because the API refuses a compressed container (`PR #186 review
    BLOCKER 1`) and a binary column. Returns its records."""
    import fastavro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [{"name": f"c{n}", "type": "string"} for n in range(columns)],
    }
    cell, rows = avro_width(total_bytes, columns)
    written = 0

    def records():
        nonlocal written
        block = 20_000
        while written < rows:
            n = min(block, rows - written)
            cells = [_random_cells(n, cell) for _ in range(columns)]
            for index in range(n):
                written += 1
                yield {f"c{k}": cells[k][index] for k in range(columns)}

    with path.open("wb") as sink:
        fastavro.writer(sink, schema, records())
    return written


def _repeating_cell(index: int, cell: int) -> str:
    """A cell of `cell` characters that repeats itself, so that a zip of them is small on
    disk: the size the cap counts is the unpacked one (feature page decision 2), and the
    unpacked size is what this measurement is about."""
    return (f"{index:012d}" + "x" * cell)[:cell]


def write_csv_zip(path: Path, unpacked_bytes: int, files: int = ZIP_FILES, columns: int = 10) -> int:
    """One zip holding `files` CSVs that unpack to at most `unpacked_bytes` bytes. Every
    entry carries the same header (the API refuses a zip whose files differ) and the
    load skips each entry's header row. Returns the rows the load reads."""
    import zipfile

    cell, _ = row_width(unpacked_bytes, columns)
    row_bytes = columns * cell + columns  # the cells, the separators and the newline
    header = _csv_header(columns)
    # The rows are shared out over the files, so that the whole zip holds as many rows as
    # one file of that size would: a run stays comparable with the other formats.
    rows = max(0, (unpacked_bytes - files * len(header)) // row_bytes)
    per_file, extra = divmod(rows, files)
    written = 0
    block = 20_000
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for number in range(files):
            count = per_file + (1 if number < extra else 0)
            with archive.open(f"part-{number}.csv", "w") as entry:
                entry.write(header.encode("utf-8"))
                for start in range(0, count, block):
                    n = min(block, count - start)
                    entry.write(
                        "".join(
                            ",".join([_repeating_cell(start + row, cell)] * columns) + "\n" for row in range(n)
                        ).encode("utf-8")
                    )
            written += count
    return written


def _string_array(rows: int, cell: int):
    """An Arrow string array of `rows` cells of `cell` characters, built from buffers: the
    text is one block and the offsets one small array, so a row group of 2 GiB does not
    need a Python list of two million strings."""
    import pyarrow as pa

    data = base64.b64encode(os.urandom(rows * cell * 3 // 4 + 3))[: rows * cell]
    offsets = array("i", range(0, (rows + 1) * cell, cell))
    return pa.StringArray.from_buffers(rows, pa.py_buffer(offsets), pa.py_buffer(data))


def write_parquet_one_group(path: Path, total_bytes: int, columns: int = 10) -> int:
    """One Parquet file of at most `total_bytes` bytes that holds exactly one row group.

    The row group is the whole file, and the reader decodes a row group whole: this is
    the file a load cannot read without holding it in memory. The size asked for bounds
    the file AND the row group it decodes to: Parquet writes a four-byte length in front
    of every value, so the row group decodes to a little less than the file.

    Returns its rows."""
    import pyarrow as pa
    import pyarrow.parquet as pq

    cell, _ = row_width(total_bytes, columns)
    per_row = columns * (4 + cell)  # every value carries its length
    schema = pa.schema([(f"c{n}", pa.string()) for n in range(columns)])
    room = min(PARQUET_FOOTER_ROOM, max(4096, total_bytes // 64))
    rows = max(1, (total_bytes - room) // per_row)
    for _ in range(3):
        table = pa.Table.from_arrays([_string_array(rows, cell) for _ in range(columns)], schema=schema)
        with path.open("wb") as sink, pq.ParquetWriter(sink, schema, compression="snappy") as writer:
            writer.write_table(table, row_group_size=rows)
        size = path.stat().st_size
        if size <= total_bytes:
            return rows
        # The file came out bigger than the size asked for (the encoding of the values
        # costs more than the estimate): fewer rows, in proportion, one more try.
        rows = max(1, rows * total_bytes // size)
    raise HarnessFailure(f"the Parquet file of one row group did not fit in {total_bytes} bytes")


# ── Sampling the containers ──────────────────────────────────────────────


def _docker_cat(container: str, path: str) -> int | None:
    try:
        out = subprocess.run(
            ["docker", "exec", container, "cat", path], capture_output=True, text=True, timeout=10, check=False
        )
        return int(out.stdout.strip()) if out.returncode == 0 and out.stdout.strip().isdigit() else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


def _docker_oom_killed(container: str) -> bool:
    """Whether the container was OOM-killed (`docker inspect .State.OOMKilled`): a job that
    died of memory is a result to record, not a harness error. The flag is the container's
    state, so it stays true until the container is replaced."""
    try:
        out = subprocess.run(
            ["docker", "inspect", "--format", "{{.State.OOMKilled}}", container],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return out.returncode == 0 and out.stdout.strip().lower() == "true"


class Sampler:
    """Reads `memory.current` of each container once a second while a phase runs.

    One window per `with`: entering drops the previous phase's maximum, records the
    baseline (the memory just before the phase) and tries to reset `memory.peak`.
    """

    def __init__(self, containers: dict[str, str], interval_s: float = 1.0) -> None:
        self.containers = containers
        self.interval_s = interval_s
        self.maxima: dict[str, int | None] = {name: None for name in containers}
        self.baselines: dict[str, int | None] = {name: None for name in containers}
        self.peak_reset: dict[str, bool] = {name: False for name in containers}
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def _sample(self) -> None:
        for name, container in self.containers.items():
            value = _docker_cat(container, "/sys/fs/cgroup/memory.current")
            if value is not None:
                self.maxima[name] = max(self.maxima[name] or 0, value)

    def _run(self) -> None:
        while not self._stop.is_set():
            self._sample()
            self._stop.wait(self.interval_s)

    def __enter__(self) -> "Sampler":
        self._stop.clear()
        self.maxima = {name: None for name in self.containers}
        self.baselines = {
            name: _docker_cat(container, "/sys/fs/cgroup/memory.current") for name, container in self.containers.items()
        }
        self.peak_reset = {name: _reset_peak(container) for name, container in self.containers.items()}
        if self.containers:
            self._thread = threading.Thread(target=self._run, daemon=True)
            self._thread.start()
        return self

    def __exit__(self, *exc) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=15)
            self._thread = None
        self._sample()  # the state at the end of the window, however short it was

    def peaks(self) -> dict[str, int | None]:
        return {name: _docker_cat(container, "/sys/fs/cgroup/memory.peak") for name, container in self.containers.items()}


def _reset_peak(container: str) -> bool:
    """Writes to `memory.peak` (cgroup v2, kernel 6.12 and later): any write resets the
    watermark of the writing file descriptor. True when the container accepted it."""
    try:
        out = subprocess.run(
            ["docker", "exec", container, "sh", "-c", "echo 0 > /sys/fs/cgroup/memory.peak"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        return out.returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


# ── The API ──────────────────────────────────────────────────────────────


def _expect(resp, status: int, what: str):
    if resp.status_code != status:
        raise HarnessFailure(f"{what}: expected {status}, got {resp.status_code} {resp.text[:300]}")
    return resp


def login(api) -> None:
    for name, value in (("AUTH_BOOTSTRAP_EMAIL", AUTH_EMAIL), ("AUTH_BOOTSTRAP_PASSWORD", AUTH_PASSWORD)):
        if not value:
            raise HarnessFailure(f"set {name}: the harness has no default credential")
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", QUERY_DB):
        raise HarnessFailure(f"ICEBERG_QUERY_DB {QUERY_DB!r} is not a plain identifier")
    resp = api.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=15)
    if not resp.ok:
        raise HarnessFailure(f"login failed: {resp.status_code} {resp.text[:300]}")
    if resp.json().get("mustChangePassword"):
        # `change_password` (routes/auth.rs) answers `StatusCode::NO_CONTENT`: 204, exactly.
        # g9-upload-large run 39795803595 expected 200 and stopped here.
        _expect(
            api.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=15),
            204,
            "forced password rotation",
        )
        _expect(
            api.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=15),
            200,
            "re-login after rotation",
        )
    me = _expect(api.get(f"{API_URL}/api/auth/me", timeout=15), 200, "GET /api/auth/me").json()
    mine = [t["id"] for t in me.get("tenants", [])]
    if TENANT_ID:
        if TENANT_ID not in mine:
            raise HarnessFailure(f"G9_TENANT_ID {TENANT_ID!r} is not one of the account's tenants")
        api.headers["X-Tenant"] = TENANT_ID
        return
    listed = _expect(api.get(f"{API_URL}/api/identity/tenants", timeout=15), 200, "list tenants").json()
    tenant = next((t for t in listed if t["slug"] == GATE_TENANT_SLUG), None)
    if tenant is None:
        raise HarnessFailure(f"tenant {GATE_TENANT_SLUG!r} does not exist")
    if tenant["id"] not in mine:
        # `add_user_to_tenant` (routes/identity.rs) answers 200 with the user, not 204;
        # run 39799040001 stopped here. ops/g9/upload_test.py accepts any `.ok` for it.
        _expect(api.put(f"{API_URL}/api/identity/users/{me['id']}/tenants/{tenant['id']}", timeout=15), 200, "join tenant")
    api.headers["X-Tenant"] = tenant["id"]


def send_in_pieces(api, path: Path, filename: str) -> dict:
    """Open a session for `path`, send its pieces (4 at a time) and complete it.
    Returns the completed upload."""
    size = path.stat().st_size
    opened = _expect(
        api.post(f"{API_URL}/api/uploads/sessions", json={"filename": filename, "sizeBytes": size}, timeout=30),
        201,
        "open a session",
    ).json()
    upload_id = opened["upload"]["id"]
    part_size, count = int(opened["partSize"]), int(opened["partCount"])

    def piece(number: int) -> None:
        with path.open("rb") as handle:
            handle.seek((number - 1) * part_size)
            body = handle.read(part_size)
        for attempt in range(8):
            resp = api.put(
                f"{API_URL}/api/uploads/{upload_id}/parts/{number}",
                data=body,
                headers={"Content-Type": "application/octet-stream"},
                timeout=300,
            )
            if resp.status_code == 429:
                time.sleep(float(resp.headers.get("Retry-After", "5")))
                continue
            if resp.status_code >= 500:
                time.sleep(min(30, 2**attempt))
                continue
            _expect(resp, 200, f"piece {number}")
            return
        raise HarnessFailure(f"piece {number} of {filename} was not accepted after 8 tries")

    with concurrent.futures.ThreadPoolExecutor(max_workers=PIECES_IN_FLIGHT) as pool:
        for future in [pool.submit(piece, n) for n in range(1, count + 1)]:
            future.result()
    return _expect(api.post(f"{API_URL}/api/uploads/{upload_id}/complete", timeout=600), 200, "complete").json()


def _table_of(table: str, index: int, total: int) -> str:
    """The table one load writes into: its own, so that the `replace` of one load does not
    take the rows of another (the loads of a phase run at the same time)."""
    return table if total == 1 else f"{table}_{index}"


def start_load(api, upload_id: str, table: str, *, dialect: bool) -> str | None:
    """Starts the load of one upload. `dialect` is True for a delimited file (a CSV, a
    gzipped one, or the CSVs of a zip), which needs its reading; Parquet, JSON Lines and
    Avro carry their own layout and send none.

    Returns `None` when the load started, or why it did not: the API refused it (too many
    loads at once, a busy table, a bad request). A refusal is a result to record, not a
    harness error."""
    body: dict = {"bronzeTable": table, "mode": "replace"}
    if dialect:
        body.update({"encoding": "utf-8", "delimiter": ",", "headerRow": 0})
    resp = api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json=body, timeout=120)
    if resp.status_code == 200:
        return None
    return f"the API refused the load: {resp.status_code} {resp.text[:200]}"


def run_loads(api, uploads: list[dict], table: str, *, dialect: bool, rows: int) -> list["Load"]:
    """Starts every load together and waits for all of them.

    The loads start together: every `POST .../ingest` is sent before any of them is
    waited for, so the loads of one phase run at the same time. A load that fails is a
    result to record and not a harness error: the other loads are waited for, and the
    report says what happened."""
    tables = [_table_of(table, index, len(uploads)) for index in range(len(uploads))]
    refusals: list[str | None] = [None] * len(uploads)
    started = time.time()

    def start(index: int) -> None:
        refusals[index] = start_load(api, uploads[index]["id"], tables[index], dialect=dialect)

    with concurrent.futures.ThreadPoolExecutor(max_workers=len(uploads)) as pool:
        for future in [pool.submit(start, index) for index in range(len(uploads))]:
            future.result()

    loads: list[Load] = []
    pending = {index for index in range(len(uploads)) if refusals[index] is None}
    for index, refusal in enumerate(refusals):
        if refusal is not None:
            loads.append(Load(index, time.time() - started, "failed", None, refusal))
    deadline = started + LOAD_TIMEOUT_S
    while pending:
        for index in sorted(pending):
            upload = _expect(
                api.get(f"{API_URL}/api/uploads/{uploads[index]['id']}", timeout=30), 200, "poll"
            ).json()
            if upload["status"] not in ("ingested", "failed"):
                continue
            ended = time.time()
            pending.discard(index)
            if upload["status"] == "failed":
                loads.append(
                    Load(index, ended - started, "failed", None, f"the load of {tables[index]} failed: {upload.get('error')}")
                )
                continue
            try:
                counted = count_rows(api, tables[index])
            except HarnessFailure as exc:
                loads.append(Load(index, ended - started, "failed", None, str(exc)))
                continue
            if counted != rows:
                loads.append(
                    Load(
                        index,
                        ended - started,
                        "failed",
                        counted,
                        f"the file held {rows:,} rows and the table holds {counted:,}",
                    )
                )
                continue
            loads.append(Load(index, ended - started, "ok", counted, ""))
        if pending:
            if time.time() >= deadline:
                for index in sorted(pending):
                    loads.append(
                        Load(
                            index,
                            time.time() - started,
                            "failed",
                            None,
                            f"the load of {tables[index]} did not finish in {LOAD_TIMEOUT_S} s",
                        )
                    )
                pending.clear()
                break
            time.sleep(5)
    return sorted(loads, key=lambda load: load.index)


def count_rows(api, table: str) -> int:
    """`WHERE 1`, never a bare count of a raw Iceberg table (R11)."""
    deadline = time.time() + VISIBLE_TIMEOUT_S
    last: Exception | None = None
    while time.time() < deadline:
        try:
            resp = api.post(
                f"{API_URL}/api/query/run",
                json={"sql": f"SELECT count() AS n FROM {QUERY_DB}.`bronze.{table}` WHERE 1"},
                timeout=120,
            )
            return int(_expect(resp, 200, "count").json()["rows"][0]["n"])
        except Exception as exc:  # noqa: BLE001 -- a table may take a beat to be visible
            last = exc
            time.sleep(3)
    raise HarnessFailure(f"could not count {table}: {last}")


# ── The report ───────────────────────────────────────────────────────────


@dataclass
class Load:
    """One load of a phase: how long it took and how it ended. A load that failed is a
    result to record and not a harness error."""

    index: int
    seconds: float
    status: str
    rows: int | None = None
    detail: str = ""


@dataclass
class Phase:
    name: str
    bytes: int
    rows: int | None
    seconds: float
    maxima: dict[str, int | None] = field(default_factory=dict)
    peaks: dict[str, int | None] = field(default_factory=dict)
    baselines: dict[str, int | None] = field(default_factory=dict)
    peak_reset: dict[str, bool] = field(default_factory=dict)
    loads: list[Load] = field(default_factory=list)
    oom: list[str] = field(default_factory=list)


def snapshot(
    name: str,
    size: int,
    rows: int | None,
    seconds: float,
    sampler: "Sampler",
    loads: list[Load] | None = None,
    oom: list[str] | None = None,
) -> Phase:
    """A phase with the figures of the sampler's window that just ended."""
    return Phase(
        name,
        size,
        rows,
        seconds,
        dict(sampler.maxima),
        sampler.peaks(),
        dict(sampler.baselines),
        dict(sampler.peak_reset),
        list(loads or []),
        list(oom or []),
    )


def _mib(value: int | None) -> str:
    return "n/a" if value is None else f"{value / MIB:,.0f}"


def _peak_text(phase: Phase, name: str) -> str:
    value = phase.peaks.get(name)
    return _mib(value) + ("*" if value is not None and phase.peak_reset.get(name) else "")


def _growth(phase: Phase, name: str) -> str:
    base, high = phase.baselines.get(name), phase.maxima.get(name)
    if base is None or high is None:
        return "n/a"
    return f"{(high - base) / MIB:+,.0f}"


def _loads_text(phase: Phase) -> str:
    """Every load of the phase: its time and how it ended."""
    if not phase.loads:
        return ""
    parts = []
    for load in phase.loads:
        rows = f", {load.rows:,} rows" if load.rows is not None else ""
        detail = f" ({load.detail})" if load.detail else ""
        parts.append(f"#{load.index + 1} {load.status} in {load.seconds:,.1f} s{rows}{detail}")
    return "; loads: " + ", ".join(parts)


def render(phases: list[Phase], names: list[str], free_bytes: int | None) -> str:
    head = (
        ["phase", "MiB", "rows", "seconds", "MiB/s", "loads"]
        + [f"{n} base" for n in names]
        + [f"{n} max" for n in names]
        + [f"{n} peak" for n in names]
    )
    lines = [head]
    for p in phases:
        rate = "n/a" if p.seconds <= 0 else f"{p.bytes / MIB / p.seconds:,.1f}"
        ok = sum(1 for load in p.loads if load.status == "ok")
        loads = f"{ok}/{len(p.loads)}" if p.loads else "-"
        lines.append(
            [p.name, f"{p.bytes / MIB:,.0f}", "n/a" if p.rows is None else f"{p.rows:,}", f"{p.seconds:,.1f}", rate, loads]
            + [_mib(p.baselines.get(n)) for n in names]
            + [_mib(p.maxima.get(n)) for n in names]
            + [_peak_text(p, n) for n in names]
        )
    widths = [max(len(row[i]) for row in lines) for i in range(len(head))]
    out = ["  ".join(cell.rjust(widths[i]) if i else cell.ljust(widths[i]) for i, cell in enumerate(row)) for row in lines]
    out.append(
        "memory in MiB: 'base' is memory.current just before the phase; 'max' is the largest memory.current sampled "
        "once a second during that phase only; 'peak' is memory.peak, '*' marking one reset at the phase start "
        "(otherwise it is since the container started)"
    )
    out.append("'loads' is how many of the loads of the phase held every row of the file, over how many loads")
    oomed = sorted({name for phase in phases for name in phase.oom})
    if oomed:
        out.append(f"containers OOM-killed during the run: {', '.join(oomed)}")
    out.append(f"free disk of the scratch directory at the end: {_mib(free_bytes)} MiB")
    return "\n".join(out)


def phase_line(phase: Phase, names: list[str], free_bytes: int | None) -> str:
    """One self-contained line for a finished phase, printed the moment it ends so that a
    failure in a later phase keeps every earlier measurement (run 39817023398 reached the
    third phase and its end-of-run table was never read)."""
    rate = "n/a" if phase.seconds <= 0 else f"{phase.bytes / MIB / phase.seconds:,.1f}"
    memory = " ".join(
        f"{n}: base {_mib(phase.baselines.get(n))} MiB, max {_mib(phase.maxima.get(n))} MiB "
        f"(growth {_growth(phase, n)} MiB), peak {_mib(phase.peaks.get(n))} MiB "
        f"({'reset at phase start' if phase.peak_reset.get(n) else 'since container start'})"
        for n in names
    )
    rows = "n/a" if phase.rows is None else f"{phase.rows:,} rows"
    if len(phase.loads) > 1:
        rows += " per load"
    oom = f"; OOM-killed: {', '.join(phase.oom)}" if phase.oom else ""
    return (
        f"[large] RESULT {phase.name}: {phase.bytes / MIB:,.0f} MiB, {rows} rows, {phase.seconds:,.1f} s, "
        f"{rate} MiB/s; {memory or 'no containers sampled'}{_loads_text(phase)}{oom}; "
        f"free disk {_mib(free_bytes)} MiB"
    )


def _free_disk(path: Path) -> int | None:
    try:
        return shutil.disk_usage(path if path.exists() else Path("/")).free
    except OSError:
        return None


def run_phase(
    api,
    name: str,
    path: Path,
    rows: int,
    table: str,
    dialect: bool,
    sampler: Sampler,
    concurrency: int = 1,
) -> Phase:
    """One phase: the file goes through the chunked transport `concurrency` times, then
    every load starts together, each into its own table, and the phase waits for all of
    them. One sampler window covers the phase, as before."""
    size = path.stat().st_size
    if concurrency > 1:
        name = f"{name} x{concurrency}"
    print(f"[large] {name}: declares {size:,} bytes ({size / MIB:,.2f} MiB)")
    started = time.time()
    with sampler:
        uploads = [send_in_pieces(api, path, path.name) for _ in range(concurrency)]
        sent = time.time()
        loads = run_loads(api, uploads, table, dialect=dialect, rows=rows)
    oom = [name_ for name_, container in sampler.containers.items() if _docker_oom_killed(container)]
    print(f"[large] {name}: sent in {sent - started:,.0f} s, loaded and counted by {time.time() - started:,.0f} s")
    print(f"[large] {name}: OOM-killed containers: {', '.join(oom) if oom else 'none'}")
    phase = snapshot(name, size * concurrency, rows, time.time() - started, sampler, loads, oom)
    print(phase_line(phase, list(sampler.containers), _free_disk(path.parent)), flush=True)
    return phase


def parse_sizes(text: str) -> list[int]:
    """`"256,512"` -> `[256, 512]`; empty text is no list (today's one size per format)."""
    try:
        sizes = [int(part) for part in text.split(",") if part.strip()]
    except ValueError as exc:
        raise HarnessFailure(f"--sizes {text!r} is not a comma-separated list of whole MiB") from exc
    if any(size <= 0 for size in sizes):
        raise HarnessFailure(f"--sizes {text!r} holds a size that is not above zero")
    return sizes


def sizes_for(flag_mb: int, sizes: list[int]) -> list[int]:
    """The sizes one format is measured at: none when its own option is 0 (skipped), the
    `--sizes` list when there is one, otherwise its own single size."""
    if flag_mb <= 0:
        return []
    return sizes or [flag_mb]


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    """The options of the harness."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--csv-mb", type=int, default=2048, help="size of the CSV in MiB (0 skips it)")
    parser.add_argument("--parquet-files", type=int, default=10)
    parser.add_argument("--parquet-mb", type=int, default=1945, help="MiB of Parquet in all (0 skips them)")
    parser.add_argument("--jsonl-mb", type=int, default=2048, help="size of the JSON Lines file in MiB (0 skips it)")
    parser.add_argument("--gzip-mb", type=int, default=2048, help="MiB a gzipped CSV unpacks to (0 skips it)")
    parser.add_argument(
        "--concurrent",
        type=int,
        default=1,
        help="how many loads of the same file run at once: the file goes through the chunked transport that many "
        "times and every load starts together (1 is one load at a time). A file of this harness is often large, so "
        "above two the third and later loads may be refused by the large-load limit (DATA-37, "
        "UPLOAD_MAX_CONCURRENT_LARGE, two by default)",
    )
    parser.add_argument("--avro-mb", type=int, default=0, help="size of the Avro file in MiB (0 skips it)")
    parser.add_argument(
        "--zip-mb", type=int, default=0, help="MiB the CSVs of one zip unpack to (0 skips it)"
    )
    parser.add_argument(
        "--parquet-one-group-mb",
        type=int,
        default=0,
        help="MiB of the Parquet file that holds exactly one row group (0 skips it)",
    )
    parser.add_argument(
        "--sizes",
        default="",
        help="comma-separated MiB list, e.g. 256,512,1024,2048: each enabled format is measured at each size "
        "(its own phase and RESULT line); the per-format size options are then ignored except for 0 = skip",
    )
    parser.add_argument("--containers", default="", help="NAME=container,NAME=container to sample")
    parser.add_argument("--scratch", default="", help="a directory for the generated files")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.concurrent < 1:
        print("[large] FAILED: --concurrent must be 1 or more", file=sys.stderr)
        return 2
    containers = dict(pair.split("=", 1) for pair in args.containers.split(",") if "=" in pair)
    try:
        import requests
    except ImportError:
        print("[large] needs `requests` (pip install requests pyarrow)", file=sys.stderr)
        return 2

    scratch = Path(args.scratch or tempfile.mkdtemp(prefix="large-upload-"))
    scratch.mkdir(parents=True, exist_ok=True)
    api = requests.Session()
    phases: list[Phase] = []
    suffix = secrets.token_hex(3)
    try:
        login(api)
        sampler = Sampler(containers)
        sizes = parse_sizes(args.sizes)
        graded = bool(sizes)
        for csv_mb in sizes_for(args.csv_mb, sizes):
            label = f"csv {csv_mb} MiB" if graded else "csv"
            path = scratch / f"large-{suffix}.csv"
            rows = write_csv(path, csv_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
            phases.append(
                run_phase(api, label, path, rows, f"g9_large_{suffix}_csv{csv_mb}", True, sampler, args.concurrent)
            )
            path.unlink()
        for jsonl_mb in sizes_for(args.jsonl_mb, sizes):
            label = f"jsonl {jsonl_mb} MiB" if graded else "jsonl"
            path = scratch / f"large-{suffix}.jsonl"
            rows = write_jsonl(path, jsonl_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
            phases.append(
                run_phase(api, label, path, rows, f"g9_large_{suffix}_jsonl{jsonl_mb}", False, sampler, args.concurrent)
            )
            path.unlink()
        for gzip_mb in sizes_for(args.gzip_mb, sizes):
            label = f"csv.gz {gzip_mb} MiB" if graded else "csv.gz"
            path = scratch / f"large-{suffix}.csv.gz"
            rows = write_csv_gz(path, gzip_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB gzipped, {rows:,} rows")
            phases.append(
                run_phase(api, label, path, rows, f"g9_large_{suffix}_gz{gzip_mb}", True, sampler, args.concurrent)
            )
            path.unlink()
        for avro_mb in sizes_for(args.avro_mb, sizes):
            label = f"avro {avro_mb} MiB" if graded else "avro"
            path = scratch / f"large-{suffix}.avro"
            rows = write_avro(path, avro_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} records")
            phases.append(
                run_phase(api, label, path, rows, f"g9_large_{suffix}_avro{avro_mb}", False, sampler, args.concurrent)
            )
            path.unlink()
        for zip_mb in sizes_for(args.zip_mb, sizes):
            label = f"zip {zip_mb} MiB" if graded else "zip"
            path = scratch / f"large-{suffix}.zip"
            rows = write_csv_zip(path, zip_mb * MIB)
            print(
                f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB on disk, "
                f"unpacks to {zip_mb} MiB, {rows:,} rows"
            )
            phases.append(
                run_phase(api, label, path, rows, f"g9_large_{suffix}_zip{zip_mb}", True, sampler, args.concurrent)
            )
            path.unlink()
        for one_group_mb in sizes_for(args.parquet_one_group_mb, sizes):
            label = f"parquet one group {one_group_mb} MiB" if graded else "parquet one group"
            path = scratch / f"large-{suffix}-onegroup.parquet"
            rows = write_parquet_one_group(path, one_group_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
            phases.append(
                run_phase(
                    api, label, path, rows, f"g9_large_{suffix}_pq1g{one_group_mb}", False, sampler, args.concurrent
                )
            )
            path.unlink()
        if args.parquet_mb > 0 and args.parquet_files > 0:
            for parquet_mb in sizes_for(args.parquet_mb, sizes):
                each = parquet_mb * MIB // args.parquet_files
                tag = f"parquet {parquet_mb} MiB" if graded else "parquet"
                for n in range(args.parquet_files):
                    path = scratch / f"large-{suffix}-{n}.parquet"
                    rows = write_parquet(path, each)
                    print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
                    phases.append(
                        run_phase(
                            api,
                            f"{tag} {n + 1}/{args.parquet_files}",
                            path,
                            rows,
                            f"g9_large_{suffix}_pq{parquet_mb}_{n}",
                            False,
                            sampler,
                            args.concurrent,
                        )
                    )
                    path.unlink()
    except HarnessFailure as exc:
        print(f"[large] FAILED: {exc}", file=sys.stderr)
        if phases:
            print(render(phases, list(containers), shutil.disk_usage(scratch).free))
        return 1
    finally:
        if not args.scratch:
            shutil.rmtree(scratch, ignore_errors=True)
    print(render(phases, list(containers), shutil.disk_usage(scratch if scratch.exists() else Path("/")).free))
    failed = [load for phase in phases for load in phase.loads if load.status != "ok"]
    if failed:
        print(
            f"[large] FAILED: {len(failed)} of {sum(len(phase.loads) for phase in phases)} loads did not hold every row",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
