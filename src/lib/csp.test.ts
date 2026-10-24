import { describe, expect, it } from "bun:test"
import {
  contentSecurityPolicy,
  contentSiteReader,
  contentSites,
  newNonce,
  sitesFromSettings,
} from "./csp"

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

describe("sitesFromSettings", () => {
  it("reads the answer's contentSites and nothing when it is not that answer", () => {
    expect(sitesFromSettings({ contentSites: ["https://a.example"] })).toEqual(["https://a.example"])
    expect(sitesFromSettings({ contentSites: [] })).toEqual([])
    expect(sitesFromSettings({ contentSites: ["https://a.example/path", "https://b.example"] })).toEqual([
      "https://b.example",
    ])
    expect(sitesFromSettings({})).toBeNull()
    expect(sitesFromSettings({ contentSites: "https://a.example" })).toBeNull()
    expect(sitesFromSettings("not JSON")).toBeNull()
    expect(sitesFromSettings(null)).toBeNull()
  })
})

describe("contentSiteReader", () => {
  /** A fetch that answers once and counts its calls. */
  function answerWith(body: unknown, init: ResponseInit = { status: 200 }) {
    const calls: { cookie: string; signal: AbortSignal }[] = []
    const fetchSettings = async (cookie: string, signal: AbortSignal) => {
      calls.push({ cookie, signal })
      return new Response(JSON.stringify(body), init)
    }
    return { calls, fetchSettings }
  }

  it("a good answer is the deployment's sites, asked with the request's cookie", async () => {
    const { calls, fetchSettings } = answerWith({ contentSites: ["https://img.example"] })
    const read = contentSiteReader(fetchSettings)
    expect(await read("session=abc")).toEqual(["https://img.example"])
    expect(calls).toHaveLength(1)
    expect(calls[0].cookie).toBe("session=abc")
  })

  it("a second page within the minute reuses the answer, whoever asks", async () => {
    const { calls, fetchSettings } = answerWith({ contentSites: ["https://img.example"] })
    let clock = 1_000
    const read = contentSiteReader(fetchSettings, { now: () => clock })
    expect(await read("session=abc")).toEqual(["https://img.example"])
    clock += 59_999
    expect(await read("session=other")).toEqual(["https://img.example"])
    expect(calls).toHaveLength(1)
    // A minute after the answer, it is asked again.
    clock += 1
    expect(await read("session=other")).toEqual(["https://img.example"])
    expect(calls).toHaveLength(2)
  })

  it("an answer with no site is a real answer and is reused", async () => {
    const { calls, fetchSettings } = answerWith({ contentSites: [] })
    const read = contentSiteReader(fetchSettings)
    expect(await read("")).toEqual([])
    expect(await read("")).toEqual([])
    expect(calls).toHaveLength(1)
  })

  it("no session (401) is no site, and is not remembered", async () => {
    const { calls, fetchSettings } = answerWith({ error: "sign in" }, { status: 401 })
    const read = contentSiteReader(fetchSettings)
    expect(await read("")).toEqual([])
    expect(await read("session=abc")).toEqual([])
    expect(calls).toHaveLength(2)
  })

  it("a body that is not the settings answer is no site, and is not remembered", async () => {
    const { calls, fetchSettings } = answerWith({ timeZone: "UTC" })
    const read = contentSiteReader(fetchSettings)
    expect(await read("session=abc")).toEqual([])
    expect(await read("session=abc")).toEqual([])
    expect(calls).toHaveLength(2)
  })

  it("a body that is not JSON is no site", async () => {
    const read = contentSiteReader(async () => new Response("<html>proxy page</html>", { status: 200 }))
    expect(await read("session=abc")).toEqual([])
  })

  it("an API that never answers times out and gives no site", async () => {
    let signal: AbortSignal | null = null
    const fetchSettings = async (_cookie: string, given: AbortSignal) => {
      signal = given
      return await new Promise<Response>((_resolve, reject) => {
        given.addEventListener("abort", () => reject(given.reason as Error))
      })
    }
    const read = contentSiteReader(fetchSettings, { timeoutMs: 5 })
    expect(await read("session=abc")).toEqual([])
    expect(signal).not.toBeNull()
    expect((signal as unknown as AbortSignal).aborted).toBe(true)
  })

  it("an unreachable API is no site", async () => {
    const read = contentSiteReader(async () => {
      throw new TypeError("fetch failed")
    })
    expect(await read("session=abc")).toEqual([])
  })
})
