import { describe, expect, it } from "bun:test"
import { NextRequest } from "next/server"
import { config, proxy, renderHeaders } from "./proxy"

describe("the PDF print view's headers (BI-1, plan D4)", () => {
  it("matches /render and /embed through SEC-47's every-page matcher", () => {
    const source = (config.matcher[0] as { source: string }).source
    const re = new RegExp(`^${source}$`)
    expect(re.test("/render/dashboard/a.b.c")).toBe(true)
    expect(re.test("/render/chart/a.b.c")).toBe(true)
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

  // PR #648 review 1: the frame must be transparent from the first paint, so
  // the layout is told on the request, not after hydration.
  it("marks a transparent embed on the request the root layout renders <html> from", async () => {
    const originalFetch = global.fetch
    global.fetch = (async () =>
      new Response(JSON.stringify({ origins: [], contentSites: [] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })) as unknown as typeof fetch
    try {
      const transparent = await proxy(
        new NextRequest("http://console.test:3000/embed/dashboard/t?background=transparent"),
      )
      // Next carries request-header overrides to the page render as
      // `x-middleware-request-<name>`; that is the header `layout.tsx` reads.
      expect(transparent.headers.get("x-middleware-request-x-embed-transparent")).toBe("1")

      const opaque = await proxy(new NextRequest("http://console.test:3000/embed/signed/a.b.c"))
      expect(opaque.headers.get("x-middleware-request-x-embed-transparent")).toBeNull()

      const consolePage = await proxy(
        new NextRequest("http://console.test:3000/dashboards?background=transparent"),
      )
      expect(consolePage.headers.get("x-middleware-request-x-embed-transparent")).toBeNull()
    } finally {
      global.fetch = originalFetch
    }
  })

  // BI-19 part D (plan D13): the chart image's view runs under the same
  // policy as the PDF view; the PNG path loosens nothing.
  it("gives /render/chart/<token> the same policy and headers as the PDF view", async () => {
    const pdf = await proxy(new NextRequest("http://console.test:3000/render/dashboard/a.b.c"))
    const image = await proxy(new NextRequest("http://console.test:3000/render/chart/a.b.c"))
    const strip = (csp: string | null) => (csp ?? "").replace(/'nonce-[^']+'/g, "'nonce-X'")
    expect(strip(image.headers.get("content-security-policy"))).toBe(
      strip(pdf.headers.get("content-security-policy")),
    )
    expect(image.headers.get("content-security-policy")).toContain("frame-ancestors 'none'")
    for (const [name, value] of Object.entries(renderHeaders())) {
      expect(image.headers.get(name)).toBe(value)
    }
  })
})
