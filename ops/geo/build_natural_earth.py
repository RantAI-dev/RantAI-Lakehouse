#!/usr/bin/env python3
"""Build the bundled world and per-country maps from Natural Earth (BI-16 C2a).

# Why this script exists (plan `2026-10-24-bi-16c-advanced-charts-maps.md`, D4)

The console draws maps from GeoJSON files under `public/geo` (no tile or
boundary server, so maps work offline and in embeds). Natural Earth ships
shapefiles that are far too large to serve as they are: the 1:10m admin-1
archive is 15 MB zipped and the files unpacked are several times that. This
script turns two Natural Earth archives into

* `world.geojson`: 1:50m admin-0, one feature per country, with `name`,
  `name_long`, `ISO_A2` and `ISO_A3` as properties, and
* `admin1/<ISO_A3>.geojson`: 1:10m admin-1, one file per country, one
  feature per state or province, with `name` and `iso_3166_2`, loaded by the
  console only when a chart picks that country.

Indonesia (`IDN`) is skipped on purpose: the console keeps its own provinces
file (`id-provinces.geojson`) as the default for it (decision D4).

# How it reads the data

The archives are downloaded by a person, never by this script or its tests
(no network here). They are untrusted input: the script opens the zip with
`zipfile`, reads only the `.shp` and `.dbf` members into memory, bounds their
size, and parses them with `struct`. It runs nothing from the archive and
extracts nothing to disk. There is no third-party dependency (no `shapely`,
no `mapshaper`), so it runs on a bare Python.

# Simplification (documented in `public/geo/README.md`)

Douglas-Peucker on each ring, with a tolerance that is a fixed fraction of
the bounding-box diagonal of the country (so Russia and Monaco both come out
at a size a browser draws quickly), clamped between `MIN_TOLERANCE` and
`MAX_TOLERANCE` degrees. Rings that collapse below a triangle are dropped,
and so are islands smaller than the tolerance, unless that would leave a
feature with no shape. Coordinates are rounded to `DECIMALS` places. The
result is for thematic maps at country or state zoom: not for measuring and
not for border disputes (Natural Earth draws de facto boundaries).

Usage::

    python3 -I ops/geo/build_natural_earth.py \\
        --world-zip ne_50m_admin_0_countries.zip \\
        --admin1-zip ne_10m_admin_1_states_provinces.zip \\
        --out public/geo
"""
from __future__ import annotations

import argparse
import json
import struct
import sys
import zipfile
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable

Point = tuple[float, float]
Ring = list[Point]

# The member of a Natural Earth archive that is read; anything else in the
# archive is ignored, never extracted.
MAX_MEMBER_BYTES = 200 * 1024 * 1024
SHP_POLYGON = 5

TOLERANCE_FRACTION = 0.0015
MIN_TOLERANCE = 0.004
MAX_TOLERANCE = 0.05
DECIMALS = 3
# Indonesia keeps its own provinces file (plan D4).
SKIPPED_ADMIN1 = frozenset({"IDN"})


class BuildError(Exception):
    """The input is not the Natural Earth shapefile this script expects."""


def _member(archive: zipfile.ZipFile, suffix: str) -> bytes:
    names = [n for n in archive.namelist() if n.lower().endswith(suffix) and "/" not in n.strip("/")]
    if len(names) != 1:
        raise BuildError(f"expected exactly one top-level *{suffix} in the archive, found {len(names)}")
    info = archive.getinfo(names[0])
    if info.file_size > MAX_MEMBER_BYTES:
        raise BuildError(f"{names[0]} is {info.file_size} bytes, over the {MAX_MEMBER_BYTES} limit")
    return archive.read(names[0])


def read_dbf(data: bytes) -> list[dict[str, str]]:
    """Records of a dBase III file as dicts of stripped strings (UTF-8)."""
    if len(data) < 32:
        raise BuildError("dbf is too short")
    count, header_len, record_len = struct.unpack("<xxxxIHH", data[:12])
    fields: list[tuple[str, int]] = []
    pos = 32
    while pos < header_len - 1 and data[pos] != 0x0D:
        name = data[pos:pos + 11].split(b"\0")[0].decode("ascii", "replace")
        fields.append((name, data[pos + 16]))
        pos += 32
    if sum(width for _, width in fields) + 1 != record_len:
        raise BuildError("dbf record length does not match its fields")
    out: list[dict[str, str]] = []
    for i in range(count):
        start = header_len + i * record_len
        record = data[start:start + record_len]
        if len(record) != record_len:
            raise BuildError("dbf is truncated")
        offset = 1
        row: dict[str, str] = {}
        for name, width in fields:
            row[name] = record[offset:offset + width].decode("utf-8", "replace").strip("\0 ")
            offset += width
        out.append(row)
    return out


def read_shp_polygons(data: bytes) -> list[list[Ring]]:
    """Per record, the rings (outer and holes, as written) of a polygon shapefile.

    A null record gives an empty list so the record count still lines up with
    the dbf.
    """
    if len(data) < 100:
        raise BuildError("shp is too short")
    if struct.unpack(">i", data[:4])[0] != 9994:
        raise BuildError("shp has the wrong magic number")
    shape_type = struct.unpack("<i", data[32:36])[0]
    if shape_type != SHP_POLYGON:
        raise BuildError(f"shp holds shape type {shape_type}, expected polygons ({SHP_POLYGON})")
    records: list[list[Ring]] = []
    pos = 100
    while pos + 8 <= len(data):
        content_words = struct.unpack(">i", data[pos + 4:pos + 8])[0]
        body = data[pos + 8:pos + 8 + content_words * 2]
        if len(body) != content_words * 2:
            raise BuildError("shp is truncated")
        pos += 8 + content_words * 2
        kind = struct.unpack("<i", body[:4])[0]
        if kind == 0:
            records.append([])
            continue
        if kind != SHP_POLYGON:
            raise BuildError(f"shp record of type {kind}, expected {SHP_POLYGON}")
        parts_n, points_n = struct.unpack("<ii", body[36:44])
        parts = list(struct.unpack(f"<{parts_n}i", body[44:44 + 4 * parts_n]))
        coords_at = 44 + 4 * parts_n
        flat = struct.unpack(f"<{points_n * 2}d", body[coords_at:coords_at + 16 * points_n])
        points = [(flat[2 * i], flat[2 * i + 1]) for i in range(points_n)]
        bounds = parts + [points_n]
        records.append([points[bounds[i]:bounds[i + 1]] for i in range(parts_n)])
    return records


def read_archive(path: Path) -> list[tuple[dict[str, str], list[Ring]]]:
    """(attributes, rings) per feature of the shapefile inside the zip at `path`."""
    with zipfile.ZipFile(path) as archive:
        attrs = read_dbf(_member(archive, ".dbf"))
        shapes = read_shp_polygons(_member(archive, ".shp"))
    if len(attrs) != len(shapes):
        raise BuildError(f"{len(attrs)} dbf records but {len(shapes)} shapes")
    return list(zip(attrs, shapes))


def signed_area(ring: Ring) -> float:
    """Shoelace area; negative for the clockwise rings shapefiles use for outers."""
    total = 0.0
    for (x1, y1), (x2, y2) in zip(ring, ring[1:] + ring[:1]):
        total += x1 * y2 - x2 * y1
    return total / 2


def simplify_line(points: Ring, tolerance: float) -> Ring:
    """Douglas-Peucker without recursion (rings can be tens of thousands of points)."""
    n = len(points)
    if n < 3:
        return list(points)
    keep = [False] * n
    keep[0] = keep[-1] = True
    stack = [(0, n - 1)]
    while stack:
        lo, hi = stack.pop()
        (x1, y1), (x2, y2) = points[lo], points[hi]
        dx, dy = x2 - x1, y2 - y1
        length = (dx * dx + dy * dy) ** 0.5
        far, far_d = -1, tolerance
        for i in range(lo + 1, hi):
            x, y = points[i]
            if length == 0:
                d = ((x - x1) ** 2 + (y - y1) ** 2) ** 0.5
            else:
                d = abs(dy * (x - x1) - dx * (y - y1)) / length
            if d > far_d:
                far, far_d = i, d
        if far >= 0:
            keep[far] = True
            stack.append((lo, far))
            stack.append((far, hi))
    return [p for p, k in zip(points, keep) if k]


def _bbox(rings: Iterable[Ring]) -> tuple[float, float, float, float]:
    xs = [x for ring in rings for x, _ in ring]
    ys = [y for ring in rings for _, y in ring]
    return min(xs), min(ys), max(xs), max(ys)


def tolerance_for(rings: Iterable[Ring]) -> float:
    """Degrees of simplification for one country: a fraction of its extent, clamped."""
    x0, y0, x1, y1 = _bbox(rings)
    diag = ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5
    return min(MAX_TOLERANCE, max(MIN_TOLERANCE, diag * TOLERANCE_FRACTION))


def _round_ring(ring: Ring) -> Ring:
    out: Ring = []
    for x, y in ring:
        p = (round(x, DECIMALS), round(y, DECIMALS))
        if not out or out[-1] != p:
            out.append(p)
    return out


def simplify_ring(ring: Ring, tolerance: float) -> Ring | None:
    """A simplified closed ring, or None when it collapsed below a triangle."""
    if len(ring) < 4:
        return None
    body = simplify_line(ring[:-1] if ring[0] == ring[-1] else ring, tolerance)
    body = _round_ring(body)
    if len(body) > 1 and body[0] == body[-1]:
        body = body[:-1]
    if len(body) < 3 or signed_area(body) == 0:
        return None
    return body + [body[0]]


def to_geometry(rings: list[Ring], tolerance: float) -> dict[str, Any] | None:
    """GeoJSON geometry from shapefile rings: outers are clockwise, holes counter-clockwise.

    Output follows RFC 7946 (outer rings counter-clockwise). An island whose
    extent is below the tolerance is dropped unless nothing else would remain.
    """
    # (polygon rings, extent in degrees of its outer ring)
    polygons: list[tuple[list[Ring], float]] = []
    largest: tuple[Ring, float] | None = None
    for ring in rings:
        if len(ring) < 4:
            continue
        is_outer = signed_area(ring) < 0
        if is_outer:
            x0, y0, x1, y1 = _bbox([ring])
            if largest is None or max(x1 - x0, y1 - y0) > largest[1]:
                largest = (ring, max(x1 - x0, y1 - y0))
        simple = simplify_ring(ring, tolerance)
        if simple is None:
            continue
        if is_outer:
            x0, y0, x1, y1 = _bbox([ring])
            polygons.append(([simple[::-1]], max(x1 - x0, y1 - y0)))
        elif polygons:
            polygons[-1][0].append(simple)
    kept = [poly for poly, extent in polygons if extent >= 2 * tolerance]
    if not kept and polygons:
        kept = [max(polygons, key=lambda p: p[1])[0]]
    if not kept and largest is not None:
        # The whole feature is smaller than the tolerance (a micro-state, one
        # islet): keep its outline rounded but not simplified rather than
        # leave a region with no shape.
        rounded = _round_ring(largest[0])
        if len(rounded) >= 4 and signed_area(rounded) != 0:
            kept = [[rounded[::-1]]]
    if not kept:
        return None
    if len(kept) == 1:
        return {"type": "Polygon", "coordinates": [[list(p) for p in r] for r in kept[0]]}
    return {
        "type": "MultiPolygon",
        "coordinates": [[[list(p) for p in r] for r in poly] for poly in kept],
    }


def _dump(path: Path, features: list[dict[str, Any]]) -> int:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps({"type": "FeatureCollection", "features": features}, ensure_ascii=False, separators=(",", ":"))
    path.write_text(text + "\n", encoding="utf-8")
    return len(text.encode("utf-8")) + 1


def _code(row: dict[str, str], *keys: str) -> str:
    """First usable code among `keys`: Natural Earth writes -99 where a code is unset."""
    for key in keys:
        value = row.get(key, "").strip()
        if value and value != "-99":
            return value.upper()
    return ""


def build_world(features: list[tuple[dict[str, str], list[Ring]]], out_dir: Path) -> int:
    """Write `world.geojson`; returns its size in bytes."""
    out: list[dict[str, Any]] = []
    for row, rings in features:
        name = row.get("NAME", "").strip()
        geometry = to_geometry(rings, tolerance_for(rings)) if rings else None
        if not name or geometry is None:
            continue
        # The `_EH` columns repair the -99 codes of France, Norway and others.
        # Where Natural Earth has no ISO code (Somaliland, Northern Cyprus, the
        # Siachen Glacier) the property is empty: its own `ADM0_A3` is not ISO
        # and is not passed off as one.
        props = {
            "name": name,
            "name_long": row.get("NAME_LONG", "").strip() or name,
            "ISO_A2": _code(row, "ISO_A2_EH", "ISO_A2"),
            "ISO_A3": _code(row, "ISO_A3_EH", "ISO_A3"),
        }
        out.append({"type": "Feature", "properties": props, "geometry": geometry})
    return _dump(out_dir / "world.geojson", out)


def build_admin1(
    features: list[tuple[dict[str, str], list[Ring]]], out_dir: Path, names: dict[str, str] | None = None
) -> dict[str, int]:
    """Write `admin1/<ISO_A3>.geojson` per country; returns the size of each file.

    When `names` is given it is filled with the country name for each code
    (Natural Earth's `admin` column), for the console's catalogue.
    """
    by_country: dict[str, list[tuple[dict[str, str], list[Ring]]]] = defaultdict(list)
    for row, rings in features:
        a3 = _code(row, "adm0_a3")
        if a3 and a3 not in SKIPPED_ADMIN1 and rings and row.get("name", "").strip():
            by_country[a3].append((row, rings))
    sizes: dict[str, int] = {}
    for a3, rows in sorted(by_country.items()):
        tolerance = tolerance_for([r for _, rings in rows for r in rings])
        out: list[dict[str, Any]] = []
        for row, rings in rows:
            geometry = to_geometry(rings, tolerance)
            if geometry is None:
                continue
            props = {"name": row["name"].strip(), "iso_3166_2": _code(row, "iso_3166_2")}
            out.append({"type": "Feature", "properties": props, "geometry": geometry})
        if out:
            sizes[a3] = _dump(out_dir / "admin1" / f"{a3}.geojson", out)
            if names is not None:
                names[a3] = rows[0][0].get("admin", "").strip() or a3
    return sizes


def catalogue_ts(names: dict[str, str]) -> str:
    """The TypeScript module listing the per-country maps, sorted by country name."""
    lines = [
        "// Generated by ops/geo/build_natural_earth.py from Natural Earth (public domain);",
        "// do not edit by hand. One entry per public/geo/admin1/<ISO_A3>.geojson (BI-16 C2a, D4).",
        "export const COUNTRY_MAPS: readonly (readonly [iso3: string, country: string])[] = [",
    ]
    for a3, name in sorted(names.items(), key=lambda kv: (kv[1].casefold(), kv[0])):
        lines.append(f"  [{json.dumps(a3)}, {json.dumps(name, ensure_ascii=False)}],")
    lines.append("];")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--world-zip", type=Path, required=True, help="ne_50m_admin_0_countries.zip")
    parser.add_argument("--admin1-zip", type=Path, required=True, help="ne_10m_admin_1_states_provinces.zip")
    parser.add_argument("--out", type=Path, required=True, help="output directory (public/geo)")
    parser.add_argument("--catalogue-ts", type=Path, help="also write the TypeScript list of country maps here")
    args = parser.parse_args(argv)
    try:
        world_bytes = build_world(read_archive(args.world_zip), args.out)
        names: dict[str, str] = {}
        sizes = build_admin1(read_archive(args.admin1_zip), args.out, names)
        if args.catalogue_ts:
            args.catalogue_ts.write_text(catalogue_ts(names), encoding="utf-8")
    except (BuildError, zipfile.BadZipFile, OSError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    print(f"world.geojson: {world_bytes} bytes")
    print(f"admin1: {len(sizes)} files, {sum(sizes.values())} bytes, largest {max(sizes.items(), key=lambda kv: kv[1])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
