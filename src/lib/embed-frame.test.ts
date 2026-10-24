import { describe, expect, it } from "bun:test"
import {
  embedFrame,
  embedPageToken,
  frameAncestorsPolicy,
  FRAME_ANCESTORS_NONE,
} from "./embed-frame"

describe("frameAncestorsPolicy", () => {
  it("an empty list is frame-ancestors 'none'", () => {
    expect(frameAncestorsPolicy([])).toBe("frame-ancestors 'none'")
  })

  it("a list is the sites, in order", () => {
    expect(frameAncestorsPolicy(["https://app.customer.example", "http://localhost:3000"])).toBe(
      "frame-ancestors https://app.customer.example http://localhost:3000",
    )
  })

  it("anything that is not a plain origin is dropped, never put in the header", () => {
    expect(
      frameAncestorsPolicy([
        "*",
        "https://*.customer.example",
        "http://app.customer.example",
        "https://a.example/path",
        "https://a.example; script-src 'unsafe-inline'",
        "'self'",
        "https://a.example\r\nX-Evil: 1",
        7,
        null,
        "https://ok.example",
      ]),
    ).toBe("frame-ancestors https://ok.example")
  })

  it("a list with nothing acceptable, or no list at all, is frame-ancestors 'none'", () => {
    expect(frameAncestorsPolicy(["*"])).toBe(FRAME_ANCESTORS_NONE)
    expect(frameAncestorsPolicy(undefined)).toBe(FRAME_ANCESTORS_NONE)
    expect(frameAncestorsPolicy("https://a.example")).toBe(FRAME_ANCESTORS_NONE)
    expect(frameAncestorsPolicy({ origins: [] })).toBe(FRAME_ANCESTORS_NONE)
  })

  it("lists no more than twenty sites, once each", () => {
    const many = Array.from({ length: 25 }, (_, i) => `https://s${i}.example`)
    expect(frameAncestorsPolicy(many).split(" ").length).toBe(1 + 20)
    expect(frameAncestorsPolicy(["https://a.example", "https://a.example"])).toBe(
      "frame-ancestors https://a.example",
    )
  })
})

describe("embedPageToken", () => {
  it("reads the signed and the public-link page", () => {
    expect(embedPageToken("/embed/signed/aaa.bbb.ccc")).toEqual({ kind: "jwt", value: "aaa.bbb.ccc" })
    expect(embedPageToken("/embed/dashboard/p_abc123")).toEqual({ kind: "token", value: "p_abc123" })
    expect(embedPageToken("/embed/dashboard/p_abc123/")).toEqual({ kind: "token", value: "p_abc123" })
  })

  it("is null for any other path", () => {
    for (const path of ["/embed", "/embed/", "/embed/signed/", "/embed/other/x", "/embed/signed/a/b", "/dashboards"]) {
      expect(embedPageToken(path)).toBeNull()
    }
  })

  it("is null for a path that is not valid percent-encoding", () => {
    expect(embedPageToken("/embed/signed/%E0%A4%A")).toBeNull()
  })
})

describe("embedFrame", () => {
  it("asks the API with the page's token and turns the answer into the header's two lists", async () => {
    const asked: unknown[] = []
    const frame = await embedFrame("/embed/signed/a.b.c", async (token) => {
      asked.push(token)
      return {
        origins: ["https://app.customer.example"],
        contentSites: ["https://img.example", "https://cdn.example:8443"],
      }
    })
    expect(asked).toEqual([{ kind: "jwt", value: "a.b.c" }])
    expect(frame).toEqual({
      origins: ["https://app.customer.example"],
      contentSites: ["https://img.example", "https://cdn.example:8443"],
    })
  })

  it("an empty answer from the API is no origin and no content site", async () => {
    expect(await embedFrame("/embed/dashboard/p_x", async () => ({ origins: [], contentSites: [] }))).toEqual({
      origins: [],
      contentSites: [],
    })
  })

  it("an origin list that is not plain origins frames nothing, whatever else came with it", async () => {
    const frame = await embedFrame("/embed/signed/a.b.c", async () => ({
      origins: ["*", "https://ok.example"],
      contentSites: ["https://img.example"],
    }))
    expect(frame.origins).toEqual(["https://ok.example"])
    expect(frame.contentSites).toEqual(["https://img.example"])
  })

  it("an unknown token is whatever the API says for it, which is the empty answer", async () => {
    // The API answers {"origins": [], "contentSites": []} for a token it does
    // not know; that is the same as a board with no sites listed.
    expect(
      await embedFrame("/embed/signed/unknown.token.x", async () => ({ origins: [], contentSites: [] })),
    ).toEqual({ origins: [], contentSites: [] })
  })

  it("a failing API is no origin and no site, not an error and not open", async () => {
    const failing = async () => {
      throw new Error("connection refused")
    }
    expect(await embedFrame("/embed/signed/a.b.c", failing)).toEqual({ origins: [], contentSites: [] })
    expect(await embedFrame("/embed/signed/a.b.c", async () => "not an object")).toEqual({
      origins: [],
      contentSites: [],
    })
    expect(await embedFrame("/embed/signed/a.b.c", async () => null)).toEqual({ origins: [], contentSites: [] })
  })

  it("a body with no content sites is no site, and a non-string entry is dropped", async () => {
    expect(await embedFrame("/embed/signed/a.b.c", async () => ({ origins: ["https://a.example"] }))).toEqual({
      origins: ["https://a.example"],
      contentSites: [],
    })
    expect(
      await embedFrame("/embed/signed/a.b.c", async () => ({ origins: [], contentSites: [7, null, "https://a.example"] })),
    ).toEqual({ origins: [], contentSites: ["https://a.example"] })
  })

  it("a path with no token never asks the API", async () => {
    let asked = false
    const frame = await embedFrame("/embed/whatever", async () => {
      asked = true
      return { origins: ["https://a.example"], contentSites: ["https://img.example"] }
    })
    expect(asked).toBe(false)
    expect(frame).toEqual({ origins: [], contentSites: [] })
  })
})
