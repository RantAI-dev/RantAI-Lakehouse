import { describe, expect, it } from "bun:test"
import { NextRequest } from "next/server"
import { config, proxy, renderHeaders } from "./proxy"

describe("the PDF print view's headers (BI-1, plan D4)", () => {
  it("matches /render and /embed through SEC-47's every-page matcher", () => {
    const source = (config.matcher[0] as { source: string }).source
    const re = new RegExp(`^${source}$`)
    expect(re.test("/render/dashboard/a.b.c")).toBe(true)
    expect(re.test("/embed/dashboard/x")).toBe(true)
    expect(re.test("/api/render/data")).toBe(false)
  })

  it("forbids framing, caching, indexing and a referrer for /render/dashboard/<token>", async () => {
    const response = await proxy(new NextRequest("http://console.test:3000/render/dashboard/a.b.c"))
    for (const [name, value] of Object.entries(renderHeaders())) {
      expect(response.headers.get(name)).toBe(value)
    }
    // SEC-47's full policy now covers the print view; it still forbids framing
    // and still lets the page post its token to its own origin.
    const csp = response.headers.get("content-security-policy") ?? ""
    expect(csp).toContain("frame-ancestors 'none'")
    expect(csp).toContain("connect-src 'self'")
    expect(csp).toContain("script-src 'self' 'nonce-")
    expect(response.headers.get("cache-control")).toBe("no-store")
    expect(response.headers.get("referrer-policy")).toBe("no-referrer")
    expect(response.headers.get("x-robots-tag")).toBe("noindex")
  })
})
