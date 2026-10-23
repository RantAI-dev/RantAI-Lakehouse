import { describe, expect, it } from "bun:test"
import { contentSecurityPolicy, contentSites, newNonce } from "./csp"

const NONCE = "c3RhYmxlbG9uZ25vbmNl"

/** The policy for a deployment that allows no content site, in production. */
const NO_SITES = [
  "default-src 'self'",
  `script-src 'self' 'nonce-${NONCE}' 'strict-dynamic'`,
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "frame-src 'none'",
  "connect-src 'self'",
  "font-src 'self' data:",
  "object-src 'none'",
  "base-uri 'self'",
  "form-action 'self'",
  "frame-ancestors 'none'",
].join("; ")

describe("contentSecurityPolicy", () => {
  it("a deployment with no content site gets the exact production policy", () => {
    expect(contentSecurityPolicy({ nonce: NONCE })).toBe(NO_SITES)
  })

  it("two content sites are in img-src and frame-src, in the order given", () => {
    expect(
      contentSecurityPolicy({
        nonce: NONCE,
        contentSites: ["https://img.example", "https://cdn.example:8443"],
      }),
    ).toBe(
      NO_SITES.replace(
        "img-src 'self' data: blob:;",
        "img-src 'self' data: blob: https://img.example https://cdn.example:8443;",
      ).replace("frame-src 'none';", "frame-src https://img.example https://cdn.example:8443;"),
    )
  })

  it("a site that is not exactly https://host[:port] never reaches the header", () => {
    const bad = [
      "https://a.example; script-src 'unsafe-inline'",
      "https://a.example b.example",
      "https://a.example'",
      "'self'",
      "http://a.example",
      "http://localhost:3000",
      "https://a.example/path",
      "https://*.a.example",
      "https://a.example@evil.example",
      "https://a.example:0",
      "https://a.example:99999",
      "https://a.example:notaport",
      "data:",
      " a.example",
      "https://",
      7,
      null,
      undefined,
      { site: "https://a.example" },
    ]
    expect(contentSecurityPolicy({ nonce: NONCE, contentSites: [...bad, "https://ok.example"] })).toBe(
      NO_SITES.replace("img-src 'self' data: blob:;", "img-src 'self' data: blob: https://ok.example;").replace(
        "frame-src 'none';",
        "frame-src https://ok.example;",
      ),
    )
    // Nothing acceptable, and a value that is not a list at all, is no site.
    expect(contentSecurityPolicy({ nonce: NONCE, contentSites: bad })).toBe(NO_SITES)
  })

  it("each site is listed once, in the normal form the API stores", () => {
    expect(contentSites(["https://Img.Example", "https://img.example", "https://B.example"])).toEqual([
      "https://img.example",
      "https://b.example",
    ])
    expect(contentSites("https://a.example")).toEqual([])
  })

  it("lists at most fifty sites, as the API stores", () => {
    const many = Array.from({ length: 60 }, (_, i) => `https://s${i}.example`)
    expect(contentSites(many)).toHaveLength(50)
    expect(contentSites(many.slice(0, 49))).toHaveLength(49)
  })

  it("an embed page's frame-ancestors is the board's own list, never 'none' just because it is an embed", () => {
    const policy = contentSecurityPolicy({
      nonce: NONCE,
      frameAncestors: ["https://app.customer.example", "http://localhost:3000"],
    })
    expect(policy.endsWith("frame-ancestors https://app.customer.example http://localhost:3000")).toBe(true)
    // A frame origin list that is not plain origins is still none.
    expect(contentSecurityPolicy({ nonce: NONCE, frameAncestors: ["*"] })).toBe(NO_SITES)
  })

  it("development adds 'unsafe-eval' and the HMR websocket; production never has them", () => {
    const dev = contentSecurityPolicy({ nonce: NONCE, development: true })
    expect(dev).toContain(`script-src 'self' 'nonce-${NONCE}' 'strict-dynamic' 'unsafe-eval';`)
    expect(dev).toContain("connect-src 'self' ws: wss:;")

    for (const policy of [
      contentSecurityPolicy({ nonce: NONCE }),
      contentSecurityPolicy({ nonce: NONCE, contentSites: ["https://a.example"] }),
      contentSecurityPolicy({ nonce: NONCE, frameAncestors: ["https://a.example"] }),
      // A value that is not exactly `true` is not development.
      contentSecurityPolicy({ nonce: NONCE, development: false }),
    ]) {
      expect(policy).not.toContain("unsafe-eval")
      expect(policy).not.toContain("ws:")
    }
  })

  it("the nonce is a fresh value every time, of the alphabet Next reads back", () => {
    const a = newNonce()
    const b = newNonce()
    expect(a).not.toBe(b)
    expect(a).toMatch(/^[A-Za-z0-9+/]{20,}={0,2}$/)
    expect(contentSecurityPolicy({ nonce: a })).toContain(`'nonce-${a}'`)
  })
})
