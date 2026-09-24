import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  activeFilterValues,
  filterParam,
  namespaceAssetsHref,
  removeFilter,
  tableFilterHref,
  toggleFilterValue,
} from "./table-filter-link"

test("filterParam selalu menyertakan filterId, karena tanpa itu chip-nya hilang", () => {
  const parsed = JSON.parse(
    filterParam([
      { id: "tier", value: ["gold"], variant: "multiSelect", operator: "inArray" },
    ])
  )
  assert.equal(parsed[0].filterId, "link0")
})

test("filterParam menghormati filterId yang sudah diberikan", () => {
  const parsed = JSON.parse(
    filterParam([
      { id: "tier", value: "gold", variant: "text", operator: "eq", filterId: "abc" },
    ])
  )
  assert.equal(parsed[0].filterId, "abc")
})

test("tableFilterHref menggabungkan search dan filters", () => {
  const href = tableFilterHref(
    "/data",
    [{ id: "layer", value: "silver", variant: "text", operator: "eq" }],
    "wisata"
  )
  const url = new URL(href, "http://x")
  assert.equal(url.searchParams.get("search"), "wisata")
  assert.equal(
    url.searchParams.get("filters"),
    '[{"id":"layer","value":"silver","variant":"text","operator":"eq","filterId":"link0"}]'
  )
})

test("tableFilterHref tanpa isi tetap mengembalikan path apa adanya", () => {
  assert.equal(tableFilterHref("/data", []), "/data")
})

test("namespaceAssetsHref menyaring kolom namespace, bukan pencarian bebas", () => {
  const url = new URL(namespaceAssetsHref("lake.bronze"), "http://x")
  assert.equal(url.pathname, "/data")
  assert.equal(url.searchParams.get("search"), null)
  const [filter] = JSON.parse(url.searchParams.get("filters") ?? "[]")
  assert.deepEqual(filter, {
    id: "namespace",
    value: "lake.bronze",
    variant: "text",
    operator: "eq",
    filterId: "link0",
  })
})

test("activeFilterValues membaca nilai satuan maupun daftar", () => {
  const one = filterParam([
    { id: "layer", value: "silver", variant: "text", operator: "eq" },
  ])
  assert.deepEqual(activeFilterValues(one, "layer"), ["silver"])

  const many = filterParam([
    { id: "layer", value: ["silver", "gold"], variant: "multiSelect", operator: "inArray" },
  ])
  assert.deepEqual(activeFilterValues(many, "layer"), ["silver", "gold"])
  assert.deepEqual(activeFilterValues(many, "tier"), [])
  assert.deepEqual(activeFilterValues("bukan json", "layer"), [])
})

test("toggleFilterValue menambah dan mencabut nilai tanpa mengganggu filter lain", () => {
  const start = filterParam([
    { id: "tier", value: ["gold"], variant: "multiSelect", operator: "inArray" },
  ])
  const added = toggleFilterValue(start, "layer", "silver")
  assert.deepEqual(activeFilterValues(added, "tier"), ["gold"])
  assert.deepEqual(activeFilterValues(added, "layer"), ["silver"])

  const both = toggleFilterValue(added, "layer", "gold")
  assert.deepEqual(activeFilterValues(both, "layer"), ["silver", "gold"])

  const removed = toggleFilterValue(both, "layer", "silver")
  assert.deepEqual(activeFilterValues(removed, "layer"), ["gold"])
})

test("toggleFilterValue mengosongkan parameter saat filter terakhir dicabut", () => {
  const only = toggleFilterValue("", "layer", "raw")
  assert.equal(toggleFilterValue(only, "layer", "raw"), "")
})

test("removeFilter hanya membuang kolom yang diminta", () => {
  const start = filterParam([
    { id: "tier", value: ["gold"], variant: "multiSelect", operator: "inArray" },
    { id: "layer", value: ["raw", "silver"], variant: "multiSelect", operator: "inArray" },
  ])
  const cleared = removeFilter(start, "layer")
  assert.deepEqual(activeFilterValues(cleared, "layer"), [])
  assert.deepEqual(activeFilterValues(cleared, "tier"), ["gold"])
})
