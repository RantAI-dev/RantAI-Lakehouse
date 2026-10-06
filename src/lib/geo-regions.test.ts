import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  buildRegionIndex,
  matchRegion,
  matchRegionRows,
  normalizeJakartaArea,
  type RegionLevel,
} from "./geo-regions"

const PROVINCES = [
  "Aceh", "Dki Jakarta", "Daerah Istimewa Yogyakarta", "Kepulauan Bangka Belitung", "Kepulauan Riau",
  "Jawa Barat", "Nusa Tenggara Timur",
]
const REGENCIES = [
  "Bandung", "Kota Bandung", "Bandung Barat", "Kota Jakarta Pusat", "Kepulauan Seribu", "Sleman",
  "Kota Yogyakarta", "Kota Baru", "Kabanjahe", "Kota Pangkal Pinang",
]
const JAKARTA = ["Jakarta Barat", "Jakarta Pusat", "Jakarta Selatan", "Jakarta Timur", "Jakarta Utara", "Kepulauan Seribu"]

const match = (raw: string, level: RegionLevel, features: string[]) =>
  matchRegion(raw, level, buildRegionIndex(features))

test("matching ignores case and collapses whitespace", () => {
  assert.equal(match("  JAWA   barat ", "province", PROVINCES), "Jawa Barat")
  assert.equal(match("bandung\tbarat", "regency", REGENCIES), "Bandung Barat")
})

test("on the regency map Kabupaten, Kab. and Kab prefixes are dropped", () => {
  for (const raw of ["Kabupaten Bandung", "Kab. Bandung", "Kab Bandung", "KAB.BANDUNG", "kabupaten  bandung"]) {
    assert.equal(match(raw, "regency", REGENCIES), "Bandung", raw)
  }
  assert.equal(match("Kabupaten Administrasi Kepulauan Seribu", "regency", REGENCIES), "Kepulauan Seribu")
})

test("a name that merely starts with the letters kab is not a prefix", () => {
  assert.equal(match("Kabanjahe", "regency", REGENCIES), "Kabanjahe")
})

test("Kota X stays Kota X, and Kota Administrasi X becomes Kota X", () => {
  assert.equal(match("Kota Bandung", "regency", REGENCIES), "Kota Bandung")
  assert.equal(match("KOTA ADMINISTRASI JAKARTA PUSAT", "regency", REGENCIES), "Kota Jakarta Pusat")
  assert.equal(match("Kota Adm. Jakarta Pusat", "regency", REGENCIES), "Kota Jakarta Pusat")
  // The kabupaten called Kotabaru is spelled with the prefix in the map.
  assert.equal(match("Kabupaten Kota Baru", "regency", REGENCIES), "Kota Baru")
})

test("a bare name is not turned into a kota, so Yogyakarta stays unmatched on the regency map", () => {
  assert.equal(match("Yogyakarta", "regency", REGENCIES), null)
  assert.equal(match("Pangkal Pinang", "regency", REGENCIES), null)
  assert.equal(match("Kota Yogyakarta", "regency", REGENCIES), "Kota Yogyakarta")
})

test("province aliases reach the bundled spelling", () => {
  assert.equal(match("DKI Jakarta", "province", PROVINCES), "Dki Jakarta")
  for (const raw of ["DI Yogyakarta", "D.I. Yogyakarta", "DI. Yogyakarta", "Yogyakarta", "Daerah Istimewa Yogyakarta"]) {
    assert.equal(match(raw, "province", PROVINCES), "Daerah Istimewa Yogyakarta", raw)
  }
  assert.equal(match("Bangka Belitung", "province", PROVINCES), "Kepulauan Bangka Belitung")
  assert.equal(match("Provinsi Jawa Barat", "province", PROVINCES), "Jawa Barat")
  assert.equal(match("NTT", "province", PROVINCES), "Nusa Tenggara Timur")
})

test("the Jakarta map keeps the behaviour normalizeJakartaArea always had", () => {
  assert.equal(normalizeJakartaArea("KOTA JAKARTA PUSAT"), "Jakarta Pusat")
  assert.equal(normalizeJakartaArea("Kab. Administrasi Kepulauan Seribu"), "Kepulauan Seribu")
  assert.equal(normalizeJakartaArea("jakarta timur"), "Jakarta Timur")
  assert.equal(match("Kota Administrasi Jakarta Selatan", "city", JAKARTA), "Jakarta Selatan")
  assert.equal(match("Bekasi", "city", JAKARTA), null)
})

test("rows for the same region are summed and the rest keep their own value", () => {
  const result = matchRegionRows(
    [
      { name: "Kab. Bandung", value: 10 },
      { name: "Kabupaten Bandung", value: 5 },
      { name: "Kota Bandung", value: 7 },
    ],
    "regency",
    REGENCIES
  )
  assert.deepEqual(result.data, [
    { name: "Bandung", value: 15 },
    { name: "Kota Bandung", value: 7 },
  ])
  assert.equal(result.unmatchedRows, 0)
})

test("rows that match no region are counted and named, not dropped silently", () => {
  const result = matchRegionRows(
    [
      { name: "Sleman", value: 1 },
      { name: "Atlantis", value: 2 },
      { name: "atlantis", value: 3 },
      { name: "Lemuria", value: 4 },
      { name: "  ", value: 5 },
    ],
    "regency",
    REGENCIES
  )
  assert.deepEqual(result.data, [{ name: "Sleman", value: 1 }])
  assert.equal(result.unmatchedRows, 4)
  assert.deepEqual(result.unmatchedNames, ["Atlantis", "atlantis", "Lemuria", "(blank)"])
})

test("a matched row with no numeric value draws nothing and is not called unmatched", () => {
  const result = matchRegionRows([{ name: "Sleman", value: Number.NaN }], "regency", REGENCIES)
  assert.deepEqual(result, { data: [], unmatchedRows: 0, unmatchedNames: [] })
})
