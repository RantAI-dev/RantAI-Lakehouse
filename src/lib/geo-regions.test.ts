import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  buildRegionIndex,
  matchRegion,
  matchRegionRows,
  rawNamesByFeature,
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

test("rawNamesByFeature lists each stored spelling once, under the feature it joins to", () => {
  const raw = ["Kab. Bandung", "Kabupaten Bandung", "Kab. Bandung", "Kota Bandung", "Nowhere"]
  const got = rawNamesByFeature(raw, "regency", REGENCIES)
  assert.deepEqual(got.get("Bandung"), ["Kab. Bandung", "Kabupaten Bandung"])
  assert.deepEqual(got.get("Kota Bandung"), ["Kota Bandung"])
  assert.equal(got.has("Nowhere"), false)
})

// BI-16 C2a (plan D4): the world map (by country) and the country maps (by state or province).
const WORLD = ["United States of America", "Côte d'Ivoire", "Czechia", "United Kingdom", "Myanmar"]
const WORLD_ALIASES: [string, string][] = [
  ["US", "United States of America"], ["USA", "United States of America"], ["United States", "United States of America"],
  ["CI", "Côte d'Ivoire"], ["Czech Republic", "Czechia"], ["GB", "United Kingdom"],
]

test("a country is found by name, long name, ISO-2 or ISO-3 code, ignoring case and accents", () => {
  const index = buildRegionIndex(WORLD, WORLD_ALIASES)
  for (const raw of ["us", "USA", "United States", "united states of america"]) {
    assert.equal(matchRegion(raw, "country", index), "United States of America", raw)
  }
  assert.equal(matchRegion("Cote d'Ivoire", "country", index), "Côte d'Ivoire")
  assert.equal(matchRegion("Czech Republic", "country", index), "Czechia")
  assert.equal(matchRegion("UK", "country", index), "United Kingdom")
  assert.equal(matchRegion("Burma", "country", index), "Myanmar")
  assert.equal(matchRegion("Atlantis", "country", index), null)
})

test("a feature name is never overridden by an alias of another feature", () => {
  const index = buildRegionIndex(["Niger", "Nigeria"], [["Niger", "Nigeria"]])
  assert.equal(matchRegion("Niger", "country", index), "Niger")
})

test("a state is found by name or ISO 3166-2 code and is not given Indonesian prefix rules", () => {
  const index = buildRegionIndex(["São Paulo", "Kota Baru", "Bandung"], [["BR-SP", "São Paulo"]])
  assert.equal(matchRegion("Sao Paulo", "subdivision", index), "São Paulo")
  assert.equal(matchRegion("br-sp", "subdivision", index), "São Paulo")
  assert.equal(matchRegion("Kabupaten Bandung", "subdivision", index), null)
  assert.equal(matchRegion("Kabupaten Bandung", "regency", index), "Bandung")
})

test("rows are summed per country across spellings, and the rest are reported", () => {
  const result = matchRegionRows(
    [{ name: "US", value: 1 }, { name: "United States", value: 2 }, { name: "Narnia", value: 4 }],
    "country", WORLD, WORLD_ALIASES
  )
  assert.deepEqual(result.data, [{ name: "United States of America", value: 3 }])
  assert.equal(result.unmatchedRows, 1)
  assert.deepEqual(result.unmatchedNames, ["Narnia"])
  const raw = rawNamesByFeature(["US", "United States", "Narnia"], "country", WORLD, WORLD_ALIASES)
  assert.deepEqual(raw.get("United States of America"), ["US", "United States"])
})
