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
   The rows are sized so that a file stays under the job's 2,000,000-row cap
   (`file_ingest.MAX_ROWS`): wide rows of random text, which also compress
   poorly, so the Parquet files reach the size asked for.
3. Sends each file in 8 MiB pieces (`POST /api/uploads/sessions`, `PUT .../parts/{n}`,
   `POST .../complete`), 4 pieces at a time, and loads it with `replace` into its
   own raw table (`POST .../ingest`), then polls the upload to `ingested`.
4. Counts the rows through the query route (`WHERE 1`, R11) and compares.
5. While it does all that, samples the memory of the containers named by
   `--containers` (`NAME=container` pairs, e.g. `api=g9large-lakehouse-api-1`,
   `dagster=g9large-dagster-code-location-1`) once a second, by
   `docker exec <container> cat /sys/fs/cgroup/memory.current`, and reads
   `memory.peak` (cgroup v2) at the end of each phase. `memory.peak` counts since
   the container started, so it never goes down; the sampled maximum is per phase.
   The Dagster run is a process of the code-location container, so that
   container's figures are the run's. The console (Next.js) is not part of this
   compose project: its proxy was measured on its own (section 8 item 3 of the plan).

# Output

One table, one row per phase: what, bytes, rows, seconds, MiB/s, then for each
container the sampled maximum and the cgroup peak in MiB, and the free disk of the
scratch directory at the end. A figure that could not be read is `n/a`; nothing
is filled in.

# Settings

`LAKEHOUSE_API_URL` (default `http://localhost:8080`: it runs on the host, not in
the compose network, because it reads the containers' cgroups), `AUTH_BOOTSTRAP_EMAIL`,
`AUTH_BOOTSTRAP_PASSWORD`, `ICEBERG_QUERY_DB` (default `icecat_api`), `G9_TENANT_ID`
(optional). Needs `requests` and `pyarrow`.
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
from dataclasses import dataclass, field
from pathlib import Path

PART_SIZE = 8 * 1024 * 1024
PIECES_IN_FLIGHT = 4
MIB = 1024 * 1024
# The job's row cap (`file_ingest.MAX_ROWS`); a generated file stays under it.
JOB_MAX_ROWS = 2_000_000
TARGET_ROWS = 1_900_000

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


def row_width(total_bytes: int, columns: int = 10) -> tuple[int, int]:
    """(characters per cell, rows) so that `columns` cells of that width, plus
    separators, make about `total_bytes` in at most `TARGET_ROWS` rows."""
    per_row = max(columns * 12, total_bytes // TARGET_ROWS + 1)
    cell = max(8, (per_row - columns) // columns)
    row_bytes = columns * cell + columns  # cells and separators/newline
    return cell, max(1, total_bytes // row_bytes)


def _random_cells(count: int, width: int) -> list[str]:
    """`count` random base64 strings of `width` characters."""
    raw = os.urandom(count * width)
    text = base64.b64encode(raw).decode("ascii")
    return [text[i * width : (i + 1) * width] for i in range(count)]


def write_csv(path: Path, total_bytes: int, columns: int = 10) -> int:
    """A CSV of about `total_bytes` bytes, written in blocks. Returns its rows."""
    cell, rows = row_width(total_bytes, columns)
    header = ",".join(f"c{n}" for n in range(columns)) + "\n"
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
    return cell, max(1, total_bytes // (columns * cell + overhead))


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
    """A gzipped CSV that unpacks to about `unpacked_bytes` bytes, streamed through
    gzip so the plain file is never on disk. Returns its rows."""
    import gzip

    cell, rows = row_width(unpacked_bytes, columns)
    header = ",".join(f"c{n}" for n in range(columns)) + "\n"
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
    """A Parquet file of about `total_bytes` bytes on disk. Returns its rows."""
    import pyarrow as pa
    import pyarrow.parquet as pq

    cell, _ = row_width(total_bytes, columns)
    schema = pa.schema([(f"c{n}", pa.string()) for n in range(columns)])
    rows = 0
    with pq.ParquetWriter(path, schema, compression="snappy") as writer:
        while path.stat().st_size < total_bytes and rows < JOB_MAX_ROWS - 50_000:
            n = 50_000
            writer.write_table(pa.table({f"c{k}": _random_cells(n, cell) for k in range(columns)}, schema=schema))
            rows += n
    return rows


# ── Sampling the containers ──────────────────────────────────────────────


def _docker_cat(container: str, path: str) -> int | None:
    try:
        out = subprocess.run(
            ["docker", "exec", container, "cat", path], capture_output=True, text=True, timeout=10, check=False
        )
        return int(out.stdout.strip()) if out.returncode == 0 and out.stdout.strip().isdigit() else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


class Sampler:
    """Reads `memory.current` of each container once a second while a phase runs."""

    def __init__(self, containers: dict[str, str], interval_s: float = 1.0) -> None:
        self.containers = containers
        self.interval_s = interval_s
        self.maxima: dict[str, int | None] = {name: None for name in containers}
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def _run(self) -> None:
        while not self._stop.is_set():
            for name, container in self.containers.items():
                value = _docker_cat(container, "/sys/fs/cgroup/memory.current")
                if value is not None:
                    self.maxima[name] = max(self.maxima[name] or 0, value)
            self._stop.wait(self.interval_s)

    def __enter__(self) -> "Sampler":
        if self.containers:
            self._thread = threading.Thread(target=self._run, daemon=True)
            self._thread.start()
        return self

    def __exit__(self, *exc) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=15)

    def peaks(self) -> dict[str, int | None]:
        return {name: _docker_cat(container, "/sys/fs/cgroup/memory.peak") for name, container in self.containers.items()}


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


def load_and_wait(api, upload_id: str, table: str, *, dialect: bool) -> dict:
    """`dialect` is True for a delimited file (a CSV, or a gzipped one), which needs its
    reading; Parquet and JSON Lines carry their own layout and send none."""
    body: dict = {"bronzeTable": table, "mode": "replace"}
    if dialect:
        body.update({"encoding": "utf-8", "delimiter": ",", "headerRow": 0})
    _expect(api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json=body, timeout=120), 200, "ingest")
    deadline = time.time() + LOAD_TIMEOUT_S
    while time.time() < deadline:
        upload = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "poll").json()
        if upload["status"] == "ingested":
            return upload
        if upload["status"] == "failed":
            raise HarnessFailure(f"the load of {table} failed: {upload.get('error')}")
        time.sleep(5)
    raise HarnessFailure(f"the load of {table} did not finish in {LOAD_TIMEOUT_S} s")


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
class Phase:
    name: str
    bytes: int
    rows: int | None
    seconds: float
    maxima: dict[str, int | None] = field(default_factory=dict)
    peaks: dict[str, int | None] = field(default_factory=dict)


def _mib(value: int | None) -> str:
    return "n/a" if value is None else f"{value / MIB:,.0f}"


def render(phases: list[Phase], names: list[str], free_bytes: int | None) -> str:
    head = ["phase", "MiB", "rows", "seconds", "MiB/s"] + [f"{n} max" for n in names] + [f"{n} peak" for n in names]
    lines = [head]
    for p in phases:
        rate = "n/a" if p.seconds <= 0 else f"{p.bytes / MIB / p.seconds:,.1f}"
        lines.append(
            [p.name, f"{p.bytes / MIB:,.0f}", "n/a" if p.rows is None else f"{p.rows:,}", f"{p.seconds:,.1f}", rate]
            + [_mib(p.maxima.get(n)) for n in names]
            + [_mib(p.peaks.get(n)) for n in names]
        )
    widths = [max(len(row[i]) for row in lines) for i in range(len(head))]
    out = ["  ".join(cell.rjust(widths[i]) if i else cell.ljust(widths[i]) for i, cell in enumerate(row)) for row in lines]
    out.append(f"memory in MiB: 'max' is the largest memory.current sampled once a second during the phase; 'peak' is the container's memory.peak since it started")
    out.append(f"free disk of the scratch directory at the end: {_mib(free_bytes)} MiB")
    return "\n".join(out)


def run_phase(api, name: str, path: Path, rows: int, table: str, dialect: bool, sampler: Sampler) -> Phase:
    size = path.stat().st_size
    started = time.time()
    with sampler:
        upload = send_in_pieces(api, path, path.name)
        sent = time.time()
        loaded = load_and_wait(api, upload["id"], table, dialect=dialect)
    counted = count_rows(api, table)
    if counted != rows:
        raise HarnessFailure(f"{table}: the file held {rows:,} rows and the table holds {counted:,}")
    if loaded.get("rows") is not None and loaded["rows"] != rows:
        raise HarnessFailure(f"{table}: the load reported {loaded['rows']:,} rows, the file held {rows:,}")
    print(f"[large] {name}: sent in {sent - started:,.0f} s, loaded and counted by {time.time() - started:,.0f} s")
    return Phase(name, size, rows, time.time() - started, dict(sampler.maxima), sampler.peaks())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--csv-mb", type=int, default=2048, help="size of the CSV in MiB (0 skips it)")
    parser.add_argument("--parquet-files", type=int, default=10)
    parser.add_argument("--parquet-mb", type=int, default=1945, help="MiB of Parquet in all (0 skips them)")
    parser.add_argument("--jsonl-mb", type=int, default=2048, help="size of the JSON Lines file in MiB (0 skips it)")
    parser.add_argument("--gzip-mb", type=int, default=2048, help="MiB a gzipped CSV unpacks to (0 skips it)")
    parser.add_argument("--containers", default="", help="NAME=container,NAME=container to sample")
    parser.add_argument("--scratch", default="", help="a directory for the generated files")
    args = parser.parse_args(argv)
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
        if args.csv_mb > 0:
            path = scratch / f"large-{suffix}.csv"
            rows = write_csv(path, args.csv_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
            phases.append(run_phase(api, "csv", path, rows, f"g9_large_{suffix}_csv", True, sampler))
            path.unlink()
        if args.jsonl_mb > 0:
            path = scratch / f"large-{suffix}.jsonl"
            rows = write_jsonl(path, args.jsonl_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
            phases.append(run_phase(api, "jsonl", path, rows, f"g9_large_{suffix}_jsonl", False, sampler))
            path.unlink()
        if args.gzip_mb > 0:
            path = scratch / f"large-{suffix}.csv.gz"
            rows = write_csv_gz(path, args.gzip_mb * MIB)
            print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB gzipped, {rows:,} rows")
            phases.append(run_phase(api, "csv.gz", path, rows, f"g9_large_{suffix}_gz", True, sampler))
            path.unlink()
        if args.parquet_mb > 0 and args.parquet_files > 0:
            each = args.parquet_mb * MIB // args.parquet_files
            for n in range(args.parquet_files):
                path = scratch / f"large-{suffix}-{n}.parquet"
                rows = write_parquet(path, each)
                print(f"[large] generated {path.name}: {path.stat().st_size / MIB:,.0f} MiB, {rows:,} rows")
                phases.append(run_phase(api, f"parquet {n + 1}/{args.parquet_files}", path, rows, f"g9_large_{suffix}_pq{n}", False, sampler))
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
    return 0


if __name__ == "__main__":
    sys.exit(main())
