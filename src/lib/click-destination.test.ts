import { strict as assert } from "node:assert"
import { test } from "node:test"
import { decodeFilters } from "./dashboard-filter-state"
import { clickProblem, dashboardDestination, isAllowedClickUrl, queryDestination, urlDestination } from "./click-destination"

const AWKWARD = ["Bali", "a b", "a&b=c", "100%", "x#y", "a/b", "é ü 日本", "$&", "'; DROP", "a+b", ""]

test("a dashboard destination carries one in-filter that the page decodes back to the value", () => {
  for (const value of AWKWARD) {
    const href = dashboardDestination("b_1", "kab", value)
    const url = new URL(href, "http://console.example")
    assert.equal(url.pathname, "/dashboards/b_1")
    assert.equal(url.hash, "", value)
    assert.deepEqual(decodeFilters(url.searchParams.get("f")), [{ column: "kab", values: [value] }], value)
  }
})

test("a board id with a slash cannot leave the dashboards path", () => {
  const href = dashboardDestination("a/../admin", "c", "v")
  assert.ok(href.startsWith("/dashboards/a%2F..%2Fadmin?"), href)
})

test("a saved query destination encodes its id", () => {
  assert.equal(queryDestination("sq-1"), "/query-studio?saved=sq-1")
  assert.equal(queryDestination("a b&c"), "/query-studio?saved=a%20b%26c")
})

test("the value is URL-encoded wherever the template puts it", () => {
  const got = urlDestination("https://example.com/search?q={value}&again={value}#{value}", "a b&c#d/é")
  assert.deepEqual(got, {
    href: "https://example.com/search?q=a%20b%26c%23d%2F%C3%A9&again=a%20b%26c%23d%2F%C3%A9#a%20b%26c%23d%2F%C3%A9",
    external: true,
  })
})

test("a value that looks like a replacement pattern stays literal", () => {
  assert.equal(urlDestination("/x/{value}", "$&$1")?.href, "/x/%24%26%241")
})

test("a path leads inside the console and an absolute address outside it", () => {
  assert.deepEqual(urlDestination("/dashboards/b_1?x={value}", "v"), { href: "/dashboards/b_1?x=v", external: false })
  assert.deepEqual(urlDestination("http://example.com/{value}", "v"), { href: "http://example.com/v", external: true })
})

test("a value cannot turn a path into another host", () => {
  const got = urlDestination("/{value}", "/evil.example")
  assert.equal(got?.href, "/%2Fevil.example")
  assert.equal(got?.external, false)
})

test("the refused forms lead nowhere", () => {
  for (const bad of [
    "javascript:alert(1)", "JAVASCRIPT:alert({value})", "data:text/html,x", "//example.com", "///example.com", "/\\example.com",
    "{value}://example.com", "https://{value}/x", "https://example.com{value}", "https://user:{value}@example.com",
    "https://", "https:example.com", "example.com", "", " https://example.com", "https://example.com/a b", "ftp://example.com",
  ]) {
    assert.equal(isAllowedClickUrl(bad), false, bad)
    assert.equal(urlDestination(bad, "v"), null, bad)
  }
})

test("the accepted forms pass", () => {
  for (const ok of [
    "https://example.com", "HTTP://Example.COM/{value}", "https://example.com:8443/x#{value}", "https://example.com?q={value}",
    "https://user@example.com/x", "/", "/{value}", "/dashboards/b_1?f={value}",
  ]) {
    assert.equal(isAllowedClickUrl(ok), true, ok)
  }
})

test("a URL over the size limit is refused", () => {
  assert.equal(isAllowedClickUrl(`https://example.com/${"a".repeat(2048)}`), false)
})

test("a click setting is saveable only when it is complete and its URL is allowed", () => {
  assert.equal(clickProblem(undefined), null)
  assert.equal(clickProblem({ kind: "dashboard", board: "b_1", column: "kab" }), null)
  assert.ok(clickProblem({ kind: "dashboard", board: "", column: "kab" }))
  assert.ok(clickProblem({ kind: "dashboard", board: "b_1", column: "" }))
  assert.equal(clickProblem({ kind: "query", id: "sq-1" }), null)
  assert.ok(clickProblem({ kind: "query", id: "" }))
  assert.equal(clickProblem({ kind: "url", url: "https://example.com/?q={value}" }), null)
  assert.ok(clickProblem({ kind: "url", url: "" }))
  assert.ok(clickProblem({ kind: "url", url: "javascript:alert(1)" }))
  assert.ok(clickProblem({ kind: "url", url: "//example.com" }))
})
