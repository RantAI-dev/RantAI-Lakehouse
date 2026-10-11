import { describe, expect, it } from "bun:test"
import { addOrigin, checkOrigin } from "./embed-origins"

describe("checkOrigin", () => {
  it("accepts https hosts with and without a port, normalised", () => {
    expect(checkOrigin("https://App.Customer.example")).toEqual({ ok: true, origin: "https://app.customer.example" })
    expect(checkOrigin(" https://b.test:0443 ")).toEqual({ ok: true, origin: "https://b.test:443" })
  })

  it("accepts http only for localhost", () => {
    expect(checkOrigin("http://localhost:3000")).toEqual({ ok: true, origin: "http://localhost:3000" })
    expect(checkOrigin("http://app.example.com").ok).toBe(false)
    expect(checkOrigin("http://127.0.0.1").ok).toBe(false)
  })

  it("refuses wildcards, a missing scheme, paths, queries, user names and bad ports", () => {
    for (const bad of [
      "https://*.example.com", "*", "example.com", "ftp://a.test", "https://", "https://a.test/",
      "https://a.test/x", "https://a.test?x=1", "https://u@a.test", "https://[::1]", "https://a.test:0",
      "https://a.test:65536", "https://a.test:", "https://-a.test", "https://a..test", "https://a_b.test",
    ]) {
      expect(checkOrigin(bad).ok).toBe(false)
    }
  })
})

describe("addOrigin", () => {
  it("appends the normalised site", () => {
    expect(addOrigin(["https://a.example"], "https://B.example")).toEqual({
      ok: true,
      origins: ["https://a.example", "https://b.example"],
    })
  })

  it("refuses a duplicate, also in another spelling", () => {
    const r = addOrigin(["https://a.example"], "https://A.example")
    expect(r.ok).toBe(false)
  })

  it("refuses a twenty-first site", () => {
    const twenty = Array.from({ length: 20 }, (_, i) => `https://s${i}.example`)
    const r = addOrigin(twenty, "https://more.example")
    expect(r).toEqual({ ok: false, message: "At most 20 sites can be allowed." })
  })

  it("passes a validation message through", () => {
    expect(addOrigin([], "*")).toEqual({ ok: false, message: "Wildcards are not accepted; list each site." })
  })
})
