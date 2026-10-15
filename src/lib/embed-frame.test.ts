import { describe, expect, it } from "bun:test"
import {
  embedFramePolicy,
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

describe("embedFramePolicy", () => {
  it("asks the API with the page's token and turns the answer into the header", async () => {
    const asked: unknown[] = []
    const policy = await embedFramePolicy("/embed/signed/a.b.c", async (token) => {
      asked.push(token)
      return ["https://app.customer.example"]
    })
    expect(asked).toEqual([{ kind: "jwt", value: "a.b.c" }])
    expect(policy).toBe("frame-ancestors https://app.customer.example")
  })

  it("an empty list from the API is frame-ancestors 'none'", async () => {
    expect(await embedFramePolicy("/embed/dashboard/p_x", async () => [])).toBe("frame-ancestors 'none'")
  })

  it("an unknown token is whatever the API says for it, which is an empty list", async () => {
    // The API answers {"origins": []} for a token it does not know; the
    // header is then 'none', the same as a board with no sites listed.
    expect(await embedFramePolicy("/embed/signed/unknown.token.x", async () => [])).toBe(FRAME_ANCESTORS_NONE)
  })

  it("a failing API is frame-ancestors 'none', not an error and not open", async () => {
    const failing = async () => {
      throw new Error("connection refused")
    }
    expect(await embedFramePolicy("/embed/signed/a.b.c", failing)).toBe(FRAME_ANCESTORS_NONE)
    expect(await embedFramePolicy("/embed/signed/a.b.c", async () => "not a list")).toBe(FRAME_ANCESTORS_NONE)
  })

  it("a path with no token never asks the API", async () => {
    let asked = false
    const policy = await embedFramePolicy("/embed/whatever", async () => {
      asked = true
      return ["https://a.example"]
    })
    expect(asked).toBe(false)
    expect(policy).toBe(FRAME_ANCESTORS_NONE)
  })
})
