"""Tests for `build_natural_earth.py` (BI-16 C2a, plan task T4).

A three-feature shapefile is built in memory and zipped into a temporary
directory; there is no network and no Natural Earth download. Run from the
repository root:

    python3 -m unittest discover -s ops/geo -p 'test_*.py'
"""
from __future__ import annotations

import io
import json
import struct
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_natural_earth as ne  # noqa: E402


def square(x: float, y: float, size: float, clockwise: bool = True) -> ne.Ring:
    pts = [(x, y), (x, y + size), (x + size, y + size), (x + size, y), (x, y)]
    return pts if clockwise else pts[::-1]


def shp(records: list[list[ne.Ring]]) -> bytes:
    body = b""
    for i, rings in enumerate(records, start=1):
        if not rings:
            content = struct.pack("<i", 0)
        else:
            points = [p for ring in rings for p in ring]
            starts, n = [], 0
            for ring in rings:
                starts.append(n)
                n += len(ring)
            xs, ys = [p[0] for p in points], [p[1] for p in points]
            content = struct.pack("<i4d2i", 5, min(xs), min(ys), max(xs), max(ys), len(rings), len(points))
            content += struct.pack(f"<{len(starts)}i", *starts)
            content += b"".join(struct.pack("<2d", *p) for p in points)
        body += struct.pack(">2i", i, len(content) // 2) + content
    header = struct.pack(">i5ii", 9994, 0, 0, 0, 0, 0, (100 + len(body)) // 2)
    header += struct.pack("<2i8d", 1000, 5, 0, 0, 0, 0, 0, 0, 0, 0)
    return header + body


def dbf(columns: list[tuple[str, int]], rows: list[list[str]]) -> bytes:
    record_len = 1 + sum(w for _, w in columns)
    header_len = 32 + 32 * len(columns) + 1
    out = struct.pack("<4xIHH20x", len(rows), header_len, record_len)
    # Field descriptors carry the width at offset 16.
    for name, width in columns:
        out += name.encode().ljust(11, b"\0") + b"C" + b"\0" * 4 + bytes([width]) + b"\0" * 15
    out += b"\r"
    for row in rows:
        out += b" " + b"".join(v.encode().ljust(w, b" ")[:w] for v, (_, w) in zip(row, columns))
    return out


def archive(path: Path, records: list[list[ne.Ring]], columns: list[tuple[str, int]], rows: list[list[str]]) -> Path:
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("fixture.shp", shp(records))
        z.writestr("fixture.dbf", dbf(columns, rows))
    return path


WORLD_COLUMNS = [("NAME", 20), ("NAME_LONG", 30), ("ISO_A2_EH", 3), ("ISO_A3_EH", 3)]
ADMIN1_COLUMNS = [("name", 20), ("iso_3166_2", 8), ("adm0_a3", 3), ("admin", 20)]


class ReadingTests(unittest.TestCase):
    def test_a_polygon_shapefile_and_its_attributes_are_read_back_feature_by_feature(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = archive(Path(tmp) / "w.zip", [[square(0, 0, 1)], []], WORLD_COLUMNS,
                           [["Aland", "Aland Islands", "AX", "ALA"], ["Nowhere", "", "", ""]])
            features = ne.read_archive(path)
        self.assertEqual(features[0][0]["NAME_LONG"], "Aland Islands")
        self.assertEqual(features[0][1], [square(0, 0, 1)])
        self.assertEqual(features[1][1], [])

    def test_an_archive_without_exactly_one_shp_is_refused_with_a_sentence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "bad.zip"
            with zipfile.ZipFile(path, "w") as z:
                z.writestr("a.dbf", dbf(WORLD_COLUMNS, []))
            with self.assertRaisesRegex(ne.BuildError, "exactly one top-level \\*.shp"):
                ne.read_archive(path)

    def test_a_file_that_is_not_a_shapefile_is_refused(self) -> None:
        with self.assertRaisesRegex(ne.BuildError, "magic number"):
            ne.read_shp_polygons(b"\0" * 120)


class SimplifyTests(unittest.TestCase):
    def test_points_closer_than_the_tolerance_to_the_line_are_dropped_and_corners_kept(self) -> None:
        line = [(0.0, 0.0), (1.0, 0.0001), (2.0, 0.0), (2.0, 2.0)]
        self.assertEqual(ne.simplify_line(line, 0.01), [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0)])

    def test_a_sliver_ring_collapses_to_none_and_a_square_survives(self) -> None:
        self.assertIsNone(ne.simplify_ring([(0, 0), (1, 0), (2, 0), (0, 0)], 0.01))
        self.assertEqual(len(ne.simplify_ring(square(0, 0, 1), 0.01) or []), 5)

    def test_output_outers_are_counter_clockwise_and_holes_attach_to_their_outer(self) -> None:
        rings = [square(0, 0, 10), square(2, 2, 2, clockwise=False), square(20, 0, 5)]
        geometry = ne.to_geometry(rings, 0.01)
        assert geometry is not None
        self.assertEqual(geometry["type"], "MultiPolygon")
        first, second = geometry["coordinates"]
        self.assertEqual(len(first), 2)
        self.assertEqual(len(second), 1)
        self.assertGreater(ne.signed_area([tuple(p) for p in first[0]]), 0)

    def test_an_island_below_the_tolerance_is_dropped_unless_it_is_all_there_is(self) -> None:
        big, speck = square(0, 0, 10), square(20, 0, 0.01)
        geometry = ne.to_geometry([big, speck], 0.05)
        assert geometry is not None
        self.assertEqual(geometry["type"], "Polygon")
        only = ne.to_geometry([speck], 0.05)
        assert only is not None
        self.assertEqual(only["type"], "Polygon")

    def test_the_tolerance_follows_the_extent_within_its_bounds(self) -> None:
        self.assertEqual(ne.tolerance_for([square(0, 0, 0.01)]), ne.MIN_TOLERANCE)
        self.assertEqual(ne.tolerance_for([square(0, 0, 500)]), ne.MAX_TOLERANCE)


class BuildTests(unittest.TestCase):
    def test_the_world_file_keeps_names_and_iso_codes_and_leaves_unknown_codes_empty(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            path = archive(Path(tmp) / "w.zip", [[square(0, 0, 1)], [square(5, 5, 1)], [square(9, 9, 1)]], WORLD_COLUMNS, [
                ["France", "France", "FR", "FRA"],
                ["Somaliland", "Somaliland", "-99", "-99"],
                ["Unnamed", "", "", ""],
            ])
            ne.build_world(ne.read_archive(path), out)
            features = json.loads((out / "world.geojson").read_text())["features"]
        props = [f["properties"] for f in features]
        self.assertEqual(props[0], {"name": "France", "name_long": "France", "ISO_A2": "FR", "ISO_A3": "FRA"})
        self.assertEqual((props[1]["ISO_A2"], props[1]["ISO_A3"]), ("", ""))

    def test_admin1_is_split_per_country_and_indonesia_is_left_to_its_own_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            path = archive(Path(tmp) / "a.zip", [[square(0, 0, 1)], [square(2, 0, 1)], [square(0, 9, 1)]], ADMIN1_COLUMNS, [
                ["Bavaria", "DE-BY", "DEU", "Germany"],
                ["Saxony", "DE-SN", "DEU", "Germany"],
                ["Bali", "ID-BA", "IDN", "Indonesia"],
            ])
            names: dict[str, str] = {}
            sizes = ne.build_admin1(ne.read_archive(path), out, names)
            written = sorted(p.name for p in (out / "admin1").iterdir())
            germany = json.loads((out / "admin1" / "DEU.geojson").read_text())["features"]
        self.assertEqual(list(sizes), ["DEU"])
        self.assertEqual(written, ["DEU.geojson"])
        self.assertEqual([f["properties"]["name"] for f in germany], ["Bavaria", "Saxony"])
        self.assertEqual(germany[0]["properties"]["iso_3166_2"], "DE-BY")
        self.assertIn('["DEU", "Germany"]', ne.catalogue_ts(names))

    def test_main_reports_a_bad_archive_without_a_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "bad.zip"
            bad.write_bytes(b"not a zip")
            err = io.StringIO()
            real, sys.stderr = sys.stderr, err
            try:
                code = ne.main(["--world-zip", str(bad), "--admin1-zip", str(bad), "--out", str(Path(tmp) / "o")])
            finally:
                sys.stderr = real
        self.assertEqual(code, 1)
        self.assertIn("error:", err.getvalue())


if __name__ == "__main__":
    unittest.main()
