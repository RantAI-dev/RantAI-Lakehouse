import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  escapeHtml,
  guessCoordinateColumns,
  POINT_LIMIT,
  POINT_SIZE_MAX,
  POINT_SIZE_MIN,
  pointLimit,
  pointSize,
  toPoints,
} from "./geo-points"

const COLS = { lat: "lat", lon: "lon", value: "visitors", label: "place" }

test("rows become points with their label and value", () => {
  const set = toPoints([{ lat: -6.2, lon: 106.8, visitors: "120", place: "Monas" }], COLS, POINT_LIMIT)
  assert.deepEqual(set, {
    points: [{ lat: -6.2, lon: 106.8, value: 120, label: "Monas" }],
    dropped: 0,
    capped: false,
  })
})

test("rows with a missing, non-numeric or out-of-range coordinate are dropped and counted", () => {
  const rows = [
    { lat: 91, lon: 10, visitors: 1 },
    { lat: -91, lon: 10, visitors: 1 },
    { lat: 10, lon: 181, visitors: 1 },
    { lat: 10, lon: -180.5, visitors: 1 },
    { lat: null, lon: 10, visitors: 1 },
    { lat: "", lon: 10, visitors: 1 },
    { lat: "north", lon: 10, visitors: 1 },
    { lat: 90, lon: -180, visitors: 1 },
    { lat: 0, lon: 0, visitors: 1 },
  ]
  const set = toPoints(rows, { lat: "lat", lon: "lon", value: "visitors" }, POINT_LIMIT)
  assert.equal(set.dropped, 7)
  assert.deepEqual(set.points.map((p) => [p.lat, p.lon]), [[90, -180], [0, 0]])
})

test("a null coordinate is not read as zero", () => {
  const set = toPoints([{ lat: null, lon: null, visitors: 1 }], { lat: "lat", lon: "lon", value: "visitors" }, POINT_LIMIT)
  assert.equal(set.points.length, 0)
  assert.equal(set.dropped, 1)
})

test("a row with no numeric value cannot be sized and is dropped", () => {
  const set = toPoints([{ lat: 1, lon: 1, visitors: null }, { lat: 1, lon: 1, visitors: "n/a" }], COLS, POINT_LIMIT)
  assert.deepEqual(set, { points: [], dropped: 2, capped: false })
})

test("capped is true only when exactly the limit came back", () => {
  const row = { lat: 1, lon: 1, visitors: 1 }
  const make = (n: number) => Array.from({ length: n }, () => row)
  const cols = { lat: "lat", lon: "lon", value: "visitors" }
  assert.equal(toPoints(make(4999), cols, POINT_LIMIT).capped, false)
  assert.equal(toPoints(make(5000), cols, POINT_LIMIT).capped, true)
  assert.equal(toPoints(make(2000), cols, pointLimit("s_1234abcd")).capped, true)
  assert.equal(pointLimit(undefined), 5000)
})

test("capped counts the rows that came back, dropped ones included", () => {
  const rows = Array.from({ length: 5000 }, () => ({ lat: 500, lon: 1, visitors: 1 }))
  const set = toPoints(rows, { lat: "lat", lon: "lon", value: "visitors" }, POINT_LIMIT)
  assert.equal(set.capped, true)
  assert.equal(set.dropped, 5000)
})

test("symbol size follows the square root of the value, clamped to 4-28 px", () => {
  assert.equal(pointSize(0, 100), POINT_SIZE_MIN)
  assert.equal(pointSize(-5, 100), POINT_SIZE_MIN)
  assert.equal(pointSize(100, 100), POINT_SIZE_MAX)
  assert.equal(pointSize(400, 100), POINT_SIZE_MAX)
  assert.equal(pointSize(25, 100), 16)
  assert.equal(pointSize(5, 0), POINT_SIZE_MIN)
})

test("coordinate columns are guessed from their names, exact names first", () => {
  assert.deepEqual(guessCoordinateColumns(["place", "lat", "lon", "visitors"]), { lat: "lat", lon: "lon" })
  assert.deepEqual(guessCoordinateColumns(["Latitude", "Longitude"]), { lat: "Latitude", lon: "Longitude" })
  assert.deepEqual(guessCoordinateColumns(["lintang", "bujur"]), { lat: "lintang", lon: "bujur" })
  assert.deepEqual(guessCoordinateColumns(["id", "lng", "lat"]), { lat: "lat", lon: "lng" })
  assert.deepEqual(guessCoordinateColumns(["pickup_lat", "lat", "long"]), { lat: "lat", lon: "long" })
  assert.deepEqual(guessCoordinateColumns(["start_lat", "start-lon"]), { lat: "start_lat", lon: "start-lon" })
})

test("a column that only contains the letters is not a coordinate", () => {
  assert.deepEqual(guessCoordinateColumns(["platform", "salary", "longest_stay", "flat"]), {
    lat: undefined,
    lon: undefined,
  })
  assert.deepEqual(guessCoordinateColumns([]), { lat: undefined, lon: undefined })
})

test("tooltip text is escaped for HTML", () => {
  assert.equal(escapeHtml(`<b onclick="x">Tom & 'Jerry'</b>`), "&lt;b onclick=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/b&gt;")
})
