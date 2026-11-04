import { strict as assert } from "node:assert"
import { test } from "node:test"
import { catalogRedirectHref } from "./catalog-redirect"

test("catalogRedirectHref sends a bare /data link to a bare /catalog", () => {
  assert.equal(catalogRedirectHref({}), "/catalog")
})

test("catalogRedirectHref keeps the search term and the filters", () => {
  const href = catalogRedirectHref({ search: "orders", filters: '[{"id":"layer"}]' })
  const url = new URL(href, "http://x")
  assert.equal(url.pathname, "/catalog")
  assert.equal(url.searchParams.get("search"), "orders")
  assert.equal(url.searchParams.get("filters"), '[{"id":"layer"}]')
})

test("catalogRedirectHref keeps the legacy q and tier names the Catalog translates", () => {
  const url = new URL(catalogRedirectHref({ q: "orders", tier: "hot" }), "http://x")
  assert.equal(url.searchParams.get("q"), "orders")
  assert.equal(url.searchParams.get("tier"), "hot")
})

test("catalogRedirectHref drops the page number, page size and view of the old page", () => {
  assert.equal(
    catalogRedirectHref({ page: "3", perPage: "50", view: "list", search: "x" }),
    "/catalog?search=x"
  )
})

test("catalogRedirectHref reads the first value of a repeated parameter", () => {
  assert.equal(catalogRedirectHref({ search: ["a", "b"] }), "/catalog?search=a")
})
