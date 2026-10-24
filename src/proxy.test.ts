import { describe, expect, it } from "bun:test"
import { NextRequest } from "next/server"
import { config, proxy, renderHeaders } from "./proxy"

describe("the PDF print view's headers (BI-1, plan D4)", () => {
  it("matches /render as well as /embed", () => {
    expect(config.matcher).toContain("/render/:path*")
    expect(config.matcher).toContain("/embed/:path*")
  })

  it("forbids framing, caching, indexing and a referrer for /render/dashboard/<token>", async () => {
    const response = await proxy(new NextRequest("http://console.test:3000/render/dashboard/a.b.c"))
    for (const [name, value] of Object.entries(renderHeaders())) {
      expect(response.headers.get(name)).toBe(value)
    }
    expect(response.headers.get("content-security-policy")).toBe("frame-ancestors 'none'")
    expect(response.headers.get("cache-control")).toBe("no-store")
    expect(response.headers.get("referrer-policy")).toBe("no-referrer")
    expect(response.headers.get("x-robots-tag")).toBe("noindex")
  })
})
