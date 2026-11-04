import { strict as assert } from "node:assert"
import { test } from "node:test"
import { parseUrlTab, urlWithTab } from "./url-tab"

const SECTIONS = ["reporting", "maps", "tag-keys"] as const

test("parseUrlTab returns the tab the parameter names", () => {
  assert.equal(parseUrlTab("maps", SECTIONS), "maps")
})

test("parseUrlTab falls back to the first tab when the parameter is missing or unknown", () => {
  assert.equal(parseUrlTab(null, SECTIONS), "reporting")
  assert.equal(parseUrlTab(undefined, SECTIONS), "reporting")
  assert.equal(parseUrlTab("sso", SECTIONS), "reporting")
})

test("urlWithTab writes a later tab and keeps the other parameters", () => {
  const url = new URL(urlWithTab("/admin/settings", "search=x", "section", "maps", SECTIONS), "http://x")
  assert.equal(url.pathname, "/admin/settings")
  assert.equal(url.searchParams.get("section"), "maps")
  assert.equal(url.searchParams.get("search"), "x")
})

test("urlWithTab leaves the first tab as the bare address", () => {
  assert.equal(urlWithTab("/admin/settings", "section=maps", "section", "reporting", SECTIONS), "/admin/settings")
  assert.equal(urlWithTab("/admin/settings", "section=maps&a=1", "section", "reporting", SECTIONS), "/admin/settings?a=1")
})
