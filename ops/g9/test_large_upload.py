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
    assert path.stat().st_size >= 2 * lu.MIB
    assert pq.ParquetFile(path).metadata.num_rows == rows


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
    assert lines[1].split().count("n/a") == 2
    assert "5,120" in lines[-1]
