"""Self-test of `large_upload.py`: its file generators, its piece sender against a
stub of the chunked routes, and its report. No stack, no network beyond loopback.
Run: `python -m pytest ops/g9/test_large_upload.py` (needs `requests`, `pyarrow`).
"""
from __future__ import annotations

import hashlib
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pyarrow.parquet as pq
import pytest
import requests

sys.path.insert(0, str(Path(__file__).parent))
import large_upload as lu  # noqa: E402


def test_rows_are_sized_to_stay_under_the_jobs_row_cap_for_any_file_size():
    for total in (50 * lu.MIB, 200 * lu.MIB, 2048 * lu.MIB):
        cell, rows = lu.row_width(total)
        assert rows <= lu.JOB_MAX_ROWS, total
        assert abs(rows * (10 * cell + 10) - total) < total * 0.02


def test_a_generated_csv_has_the_size_and_row_count_it_reports(tmp_path):
    path = tmp_path / "a.csv"
    rows = lu.write_csv(path, 3 * lu.MIB)
    lines = path.read_text(encoding="utf-8").splitlines()
    assert len(lines) == rows + 1
    assert lines[0] == ",".join(f"c{n}" for n in range(10))
    assert all(len(line.split(",")) == 10 for line in lines[1:50])
    assert abs(path.stat().st_size - 3 * lu.MIB) < 0.05 * 3 * lu.MIB


def test_a_generated_parquet_file_reaches_its_size_and_holds_the_rows_it_reports(tmp_path):
    path = tmp_path / "a.parquet"
    rows = lu.write_parquet(path, 2 * lu.MIB)
    assert 0.5 * 2 * lu.MIB <= path.stat().st_size <= 2 * lu.MIB
    assert pq.ParquetFile(path).metadata.num_rows == rows


def test_json_lines_rows_are_sized_to_stay_under_the_jobs_row_cap_for_any_file_size():
    for total in (50 * lu.MIB, 200 * lu.MIB, 2048 * lu.MIB):
        cell, rows = lu.jsonl_width(total)
        assert rows <= lu.JOB_MAX_ROWS, total
        assert abs(rows * (10 * cell + 10 * len('"c0":"",') + 2) - total) < total * 0.02


def test_a_generated_json_lines_file_has_the_size_and_row_count_it_reports(tmp_path):
    path = tmp_path / "a.jsonl"
    rows = lu.write_jsonl(path, 3 * lu.MIB)
    lines = path.read_text(encoding="utf-8").splitlines()
    assert len(lines) == rows
    first = json.loads(lines[0])
    assert list(first) == [f"c{n}" for n in range(10)]
    assert all(isinstance(v, str) for v in first.values())
    assert abs(path.stat().st_size - 3 * lu.MIB) < 0.05 * 3 * lu.MIB


def test_a_generated_gzipped_csv_unpacks_to_the_size_and_rows_it_reports(tmp_path):
    import gzip

    path = tmp_path / "a.csv.gz"
    rows = lu.write_csv_gz(path, 3 * lu.MIB)
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        text = handle.read()
    assert len(text.splitlines()) == rows + 1
    assert abs(len(text.encode("utf-8")) - 3 * lu.MIB) < 0.05 * 3 * lu.MIB
    assert path.stat().st_size < len(text)


class _Stub(BaseHTTPRequestHandler):
    sessions: dict = {}

    def _send(self, status, body=None):
        data = json.dumps(body or {}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        if self.path == "/api/uploads/sessions":
            size = body["sizeBytes"]
            part = 1024
            _Stub.sessions["up-1"] = {"parts": {}, "size": size}
            return self._send(201, {"upload": {"id": "up-1"}, "partSize": part, "partCount": -(-size // part)})
        if self.path == "/api/uploads/up-1/complete":
            session = _Stub.sessions["up-1"]
            whole = b"".join(session["parts"][n] for n in sorted(session["parts"]))
            return self._send(200, {"id": "up-1", "sizeBytes": len(whole), "sha": hashlib.sha256(whole).hexdigest()})
        self._send(404)

    def do_PUT(self):
        length = int(self.headers.get("Content-Length", "0"))
        data = self.rfile.read(length)
        number = int(self.path.rsplit("/", 1)[1])
        _Stub.sessions["up-1"]["parts"][number] = data
        self._send(200, {"partNumber": number})

    def log_message(self, *args):
        pass


def test_the_piece_sender_sends_every_piece_in_order_and_completes(tmp_path, monkeypatch):
    server = ThreadingHTTPServer(("127.0.0.1", 0), _Stub)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        monkeypatch.setattr(lu, "API_URL", f"http://127.0.0.1:{server.server_port}")
        path = tmp_path / "x.csv"
        payload = bytes(range(256)) * 41  # 10,496 bytes: ten pieces of 1,024 and a short last one
        path.write_bytes(payload)
        done = lu.send_in_pieces(requests.Session(), path, "x.csv")
        assert done["sizeBytes"] == len(payload)
        assert done["sha"] == hashlib.sha256(payload).hexdigest()
    finally:
        server.shutdown()


def test_the_report_has_one_row_per_phase_and_says_na_for_what_it_could_not_read():
    phases = [
        lu.Phase("csv", 200 * lu.MIB, 1_000_000, 40.0, {"api": 300 * lu.MIB, "dagster": None}, {"api": 400 * lu.MIB, "dagster": None}),
    ]
    text = lu.render(phases, ["api", "dagster"], 5 * 1024 * lu.MIB)
    lines = text.splitlines()
    assert lines[0].split()[:5] == ["phase", "MiB", "rows", "seconds", "MiB/s"]
    assert "200" in lines[1] and "1,000,000" in lines[1] and "5.0" in lines[1]
    assert lines[1].split().count("n/a") == 4  # api and dagster base (no baseline given), dagster max and peak
    assert "5,120" in lines[-1]


def test_a_finished_phase_prints_its_own_result_line_with_memory_and_free_disk():
    phase = lu.Phase("csv", 200 * lu.MIB, 1_000_000, 40.0, {"api": 300 * lu.MIB, "dagster": None}, {"api": 400 * lu.MIB, "dagster": None})
    line = lu.phase_line(phase, ["api", "dagster"], 5 * 1024 * lu.MIB)
    assert "\n" not in line
    assert line.startswith("[large] RESULT csv:")
    assert "1,000,000 rows" in line and "40.0 s" in line and "5.0 MiB/s" in line
    assert "api: base n/a MiB, max 300 MiB (growth n/a MiB), peak 400 MiB (since container start)" in line
    assert "dagster: base n/a MiB, max n/a MiB (growth n/a MiB), peak n/a MiB (since container start)" in line
    assert "free disk 5,120 MiB" in line


@pytest.mark.parametrize("target", [1 * lu.MIB, 3 * lu.MIB, 7 * lu.MIB + 12_345])
def test_no_generated_file_is_larger_than_the_size_asked_for(tmp_path, target):
    """g9-upload-large run 39803084936: a 2048 MiB CSV came out a header over the API's cap."""
    import gzip

    csv = tmp_path / "a.csv"
    lu.write_csv(csv, target)
    assert csv.stat().st_size <= target
    jsonl = tmp_path / "a.jsonl"
    lu.write_jsonl(jsonl, target)
    assert jsonl.stat().st_size <= target
    parquet = tmp_path / "a.parquet"
    lu.write_parquet(parquet, target)
    assert 0.5 * target <= parquet.stat().st_size <= target
    gz = tmp_path / "a.csv.gz"
    lu.write_csv_gz(gz, target)
    with gzip.open(gz, "rb") as handle:
        unpacked = sum(len(chunk) for chunk in iter(lambda: handle.read(1 << 20), b""))
    assert unpacked <= target


def test_the_row_arithmetic_never_exceeds_the_target_at_the_planned_sizes():
    for total in (50 * lu.MIB, 200 * lu.MIB, 2048 * lu.MIB):
        cell, rows = lu.row_width(total)
        assert len(lu._csv_header(10)) + rows * (10 * cell + 10) <= total
        cell, rows = lu.jsonl_width(total)
        assert rows * (10 * cell + 10 * len('"c0":"",') + 2) <= total


def test_a_phase_line_shows_the_baseline_the_growth_and_whether_the_peak_was_reset():
    phase = lu.Phase(
        "csv 256 MiB", 256 * lu.MIB, 10, 5.0, {"api": 900 * lu.MIB}, {"api": 950 * lu.MIB}, {"api": 400 * lu.MIB}, {"api": True}
    )
    line = lu.phase_line(phase, ["api"], None)
    assert "api: base 400 MiB, max 900 MiB (growth +500 MiB), peak 950 MiB (reset at phase start)" in line
    assert lu.render([phase], ["api"], None).splitlines()[1].split()[-3:] == ["400", "900", "950*"]


def test_the_sizes_list_applies_to_each_enabled_format_and_a_zero_skips_one():
    assert lu.parse_sizes("256, 512,1024,2048") == [256, 512, 1024, 2048]
    assert lu.parse_sizes("") == []
    assert lu.sizes_for(2048, [256, 512]) == [256, 512]
    assert lu.sizes_for(2048, []) == [2048]  # no list: today's one size
    assert lu.sizes_for(0, [256, 512]) == []
    for bad in ("0", "-5", "a,b"):
        with pytest.raises(lu.HarnessFailure):
            lu.parse_sizes(bad)


def test_each_phase_window_starts_empty_with_a_baseline_and_a_peak_reset(monkeypatch):
    """g9-upload-large run 39891862222: three phases printed one identical maximum."""
    readings = {"/sys/fs/cgroup/memory.current": [100, 700, 300, 120, 200, 150]}
    calls = {"reset": 0}

    def fake_cat(container, path):
        values = readings.get(path)
        return values.pop(0) if values else 150

    monkeypatch.setattr(lu, "_docker_cat", fake_cat)
    monkeypatch.setattr(lu, "_reset_peak", lambda container: calls.__setitem__("reset", calls["reset"] + 1) or calls["reset"] == 1)
    sampler = lu.Sampler({"api": "c"}, interval_s=0.01)
    with sampler:
        pass
    first_baseline, first_max = sampler.baselines["api"], sampler.maxima["api"]
    assert first_baseline == 100 and first_max is not None and sampler.peak_reset == {"api": True}
    monkeypatch.setattr(lu, "_docker_cat", lambda container, path: 50)
    with sampler:
        pass
    assert sampler.baselines["api"] == 50 and sampler.maxima["api"] == 50  # nothing carried over
    assert sampler.peak_reset == {"api": False}


# ── DATA-33 and DATA-34: the concurrent loads and the formats not yet measured ──


def test_the_harness_loads_once_and_skips_the_three_new_formats_by_default():
    args = lu.parse_args([])
    assert args.concurrent == 1
    assert args.avro_mb == 0
    assert args.zip_mb == 0
    assert args.parquet_one_group_mb == 0


def test_the_harness_reads_the_concurrency_and_the_new_sizes_from_the_command_line():
    args = lu.parse_args(["--concurrent", "4", "--avro-mb", "128", "--zip-mb", "64", "--parquet-one-group-mb", "32"])
    assert args.concurrent == 4
    assert args.avro_mb == 128
    assert args.zip_mb == 64
    assert args.parquet_one_group_mb == 32


def test_the_harness_refuses_a_concurrency_below_one(capsys):
    assert lu.main(["--concurrent", "0"]) == 2
    assert "--concurrent" in capsys.readouterr().err


def test_a_new_format_is_measured_at_the_sizes_of_the_list_and_is_skipped_when_it_is_zero():
    assert lu.sizes_for(0, [256, 512]) == []
    assert lu.sizes_for(2048, [256, 512]) == [256, 512]
    assert lu.sizes_for(2048, []) == [2048]


def test_the_generated_avro_file_is_readable_and_holds_the_records_it_reports(tmp_path):
    import fastavro

    path = tmp_path / "a.avro"
    records = lu.write_avro(path, 1 * lu.MIB)
    assert 0 < records <= lu.JOB_MAX_ROWS
    assert path.stat().st_size <= 1 * lu.MIB
    with path.open("rb") as handle:
        reader = fastavro.reader(handle)
        assert reader.metadata["avro.codec"] == "null"
        read = list(reader)
    assert len(read) == records
    assert list(read[0]) == [f"c{n}" for n in range(10)]
    assert all(isinstance(value, str) for value in read[0].values())


def test_the_generated_zip_unpacks_to_about_the_size_and_holds_the_rows_it_reports(tmp_path):
    import zipfile

    path = tmp_path / "a.zip"
    rows = lu.write_csv_zip(path, 1 * lu.MIB)
    unpacked = 0
    read_rows = 0
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        for name in names:
            data = archive.read(name)
            unpacked += len(data)
            lines = data.decode("utf-8").splitlines()
            assert lines[0] == ",".join(f"c{n}" for n in range(10))
            read_rows += len(lines) - 1
    assert len(names) == lu.ZIP_FILES  # the API reads ten files at most
    assert abs(unpacked - 1 * lu.MIB) < 0.05 * 1 * lu.MIB
    assert read_rows == rows


def test_the_generated_parquet_file_of_one_group_holds_exactly_one_row_group(tmp_path):
    path = tmp_path / "a.parquet"
    rows = lu.write_parquet_one_group(path, 1 * lu.MIB)
    metadata = pq.ParquetFile(path).metadata
    assert metadata.num_row_groups == 1
    assert metadata.num_rows == rows
    assert path.stat().st_size <= 1 * lu.MIB


def test_the_generated_files_are_never_larger_than_the_size_asked_for(tmp_path):
    """The API refuses a session above its cap, so a generated file may not cross it."""
    avro = tmp_path / "a.avro"
    lu.write_avro(avro, 1 * lu.MIB)
    assert avro.stat().st_size <= 1 * lu.MIB
    zip_file = tmp_path / "a.zip"
    lu.write_csv_zip(zip_file, 1 * lu.MIB)
    import zipfile

    with zipfile.ZipFile(zip_file) as archive:
        unpacked = sum(info.file_size for info in archive.infolist())
    assert unpacked <= 1 * lu.MIB


class _LoadStub(BaseHTTPRequestHandler):
    """The ingest and the poll of two loads: one succeeds, one is refused."""

    calls: list = []

    def _send(self, status, body=None):
        data = json.dumps(body or {}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        _LoadStub.calls.append(f"POST {self.path}")
        if self.path == "/api/uploads/up-1/ingest":
            return self._send(200, {"upload": {"id": "up-1"}})
        if self.path == "/api/uploads/up-2/ingest":
            return self._send(429, {"error": "too many loads at once"})
        if self.path == "/api/query/run":
            return self._send(200, {"rows": [{"n": 10}]})
        self._send(404)

    def do_GET(self):
        _LoadStub.calls.append(f"GET {self.path}")
        if self.path == "/api/uploads/up-1":
            return self._send(200, {"id": "up-1", "status": "ingested"})
        self._send(404)

    def log_message(self, *args):
        pass


def test_the_loads_of_a_phase_start_together_and_a_load_that_fails_is_recorded(monkeypatch):
    server = ThreadingHTTPServer(("127.0.0.1", 0), _LoadStub)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    _LoadStub.calls = []
    try:
        monkeypatch.setattr(lu, "API_URL", f"http://127.0.0.1:{server.server_port}")
        loads = lu.run_loads(
            requests.Session(), [{"id": "up-1"}, {"id": "up-2"}], "t", dialect=True, rows=10
        )
    finally:
        server.shutdown()
    assert [load.status for load in loads] == ["ok", "failed"]
    assert loads[0].rows == 10
    assert "429" in loads[1].detail
    # Every load is started before any of them is waited for.
    ingests = [index for index, call in enumerate(_LoadStub.calls) if call.endswith("/ingest")]
    polls = [index for index, call in enumerate(_LoadStub.calls) if call.startswith("GET ")]
    assert len(ingests) == 2 and polls
    assert max(ingests) < min(polls)


def test_a_phase_line_names_every_load_with_its_time_and_its_result():
    phase = lu.Phase(
        "csv x2",
        400 * lu.MIB,
        10,
        5.0,
        {},
        {},
        {},
        {},
        [lu.Load(0, 4.0, "ok", 10), lu.Load(1, 2.0, "failed", None, "the load failed")],
    )
    line = lu.phase_line(phase, [], None)
    assert "#1 ok in 4.0 s, 10 rows" in line
    assert "#2 failed in 2.0 s (the load failed)" in line
    assert "10 rows per load" in line


def test_the_report_counts_the_loads_that_held_every_row():
    phase = lu.Phase(
        "csv x2",
        400 * lu.MIB,
        10,
        5.0,
        {},
        {},
        {},
        {},
        [lu.Load(0, 4.0, "ok", 10), lu.Load(1, 2.0, "failed", None, "the load failed")],
    )
    row = lu.render([phase], [], None).splitlines()[1].split()
    assert row[-1] == "1/2"


def test_the_oom_check_reads_the_state_of_the_container(monkeypatch):
    class _Done:
        returncode = 0
        stdout = "true\n"

    monkeypatch.setattr(lu.subprocess, "run", lambda *args, **kwargs: _Done())
    assert lu._docker_oom_killed("g9large-dagster-code-location-1") is True

    class _Alive:
        returncode = 0
        stdout = "false\n"

    monkeypatch.setattr(lu.subprocess, "run", lambda *args, **kwargs: _Alive())
    assert lu._docker_oom_killed("g9large-dagster-code-location-1") is False


class _PhaseStub(BaseHTTPRequestHandler):
    """The whole route of one phase: the chunked transport, the ingest, the poll and the
    count. Every upload it serves ends up `ingested`."""

    sessions: dict = {}
    counter = 0

    def _send(self, status, body=None):
        data = json.dumps(body or {}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        if self.path == "/api/uploads/sessions":
            _PhaseStub.counter += 1
            name = f"up-{_PhaseStub.counter}"
            _PhaseStub.sessions[name] = {"parts": {}, "size": body["sizeBytes"], "table": None}
            return self._send(201, {"upload": {"id": name}, "partSize": 1024, "partCount": -(-body["sizeBytes"] // 1024)})
        if self.path.startswith("/api/uploads/") and self.path.endswith("/ingest"):
            name = self.path.split("/")[3]
            _PhaseStub.sessions[name]["table"] = body["bronzeTable"]
            return self._send(200, {"upload": {"id": name}})
        if self.path.startswith("/api/uploads/") and self.path.endswith("/complete"):
            name = self.path.split("/")[3]
            session = _PhaseStub.sessions[name]
            whole = b"".join(session["parts"][n] for n in sorted(session["parts"]))
            return self._send(200, {"id": name, "sizeBytes": len(whole), "sha": hashlib.sha256(whole).hexdigest()})
        if self.path == "/api/query/run":
            return self._send(200, {"rows": [{"n": 3}]})
        self._send(404)

    def do_PUT(self):
        length = int(self.headers.get("Content-Length", "0"))
        data = self.rfile.read(length)
        name, number = self.path.split("/")[3], int(self.path.rsplit("/", 1)[1])
        _PhaseStub.sessions[name]["parts"][number] = data
        self._send(200, {"partNumber": number})

    def do_GET(self):
        if self.path.startswith("/api/uploads/"):
            return self._send(200, {"id": self.path.split("/")[3], "status": "ingested"})
        self._send(404)

    def log_message(self, *args):
        pass


def test_a_phase_sends_the_file_loads_it_and_reports_the_load(tmp_path, monkeypatch):
    server = ThreadingHTTPServer(("127.0.0.1", 0), _PhaseStub)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    _PhaseStub.sessions = {}
    _PhaseStub.counter = 0
    try:
        monkeypatch.setattr(lu, "API_URL", f"http://127.0.0.1:{server.server_port}")
        path = tmp_path / "a.csv"
        path.write_text("c0,c1\n1,2\n3,4\n5,6\n")
        phase = lu.run_phase(
            requests.Session(), "csv", path, 3, "g9_large_test_csv", True, lu.Sampler({}), concurrency=2
        )
    finally:
        server.shutdown()
    assert len(phase.loads) == 2
    assert all(load.status == "ok" and load.rows == 3 for load in phase.loads)
    assert phase.name == "csv x2"
    assert phase.bytes == 2 * path.stat().st_size
    # Every load wrote into its own table.
    assert {session["table"] for session in _PhaseStub.sessions.values()} == {
        "g9_large_test_csv_0",
        "g9_large_test_csv_1",
    }
