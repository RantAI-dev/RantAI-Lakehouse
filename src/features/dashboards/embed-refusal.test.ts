import { describe, expect, it } from "bun:test"
import {
  EMBED_NOT_CONFIGURED,
  EMBED_TOKEN_REFUSED,
  EMBED_UNAVAILABLE,
  embedRefusalMessage,
} from "./embed-refusal"

describe("embedRefusalMessage", () => {
  it("a refused token reads the one fixed message", () => {
    expect(embedRefusalMessage(401, { error: "embed token is invalid or expired" })).toBe(EMBED_TOKEN_REFUSED)
  })

  it("a server without a secret says embedding is not configured", () => {
    expect(embedRefusalMessage(503, { error: "embedding is not configured" })).toBe(EMBED_NOT_CONFIGURED)
  })

  it("every other answer is the plain unavailable line, whatever its text", () => {
    expect(embedRefusalMessage(403, { error: "embedding_disabled" })).toBe(EMBED_UNAVAILABLE)
    expect(embedRefusalMessage(404, { error: "not_found" })).toBe(EMBED_UNAVAILABLE)
    expect(embedRefusalMessage(401, { error: "something the page never wrote" })).toBe(EMBED_UNAVAILABLE)
    expect(embedRefusalMessage(401, null)).toBe(EMBED_UNAVAILABLE)
    expect(embedRefusalMessage(503, "oops")).toBe(EMBED_UNAVAILABLE)
  })
})
