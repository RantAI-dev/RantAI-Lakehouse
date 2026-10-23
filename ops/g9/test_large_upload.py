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
