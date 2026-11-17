import { describe, expect, it } from "bun:test"
import { DEFAULT_EMBED_LOOK, parseEmbedLook, themeFor } from "./embed-look"

describe("the look of an embed (BI-26 D6)", () => {
  it("defaults to the dashboard's own look, the name shown, an opaque background", () => {
    expect(parseEmbedLook(undefined)).toEqual(DEFAULT_EMBED_LOOK)
    expect(parseEmbedLook({})).toEqual(DEFAULT_EMBED_LOOK)
  })

  it("reads theme, logo and background from the address", () => {
    expect(parseEmbedLook({ theme: "dark", logo: "none", background: "transparent" })).toEqual({
      theme: "dark", showName: false, transparent: true,
    })
    expect(parseEmbedLook({ theme: ["light", "dark"] }).theme).toBe("light")
  })

  it("ignores values it does not know", () => {
    expect(parseEmbedLook({ theme: "neon", logo: "hide", background: "red" })).toEqual(DEFAULT_EMBED_LOOK)
  })

  it("lays light or dark over the dashboard's theme and leaves 'dashboard' alone", () => {
    expect(themeFor("dark", { ...DEFAULT_EMBED_LOOK, theme: "light" })).toBe("light")
    expect(themeFor("dark", DEFAULT_EMBED_LOOK)).toBe("dark")
    expect(themeFor(undefined, { ...DEFAULT_EMBED_LOOK, theme: "dark" })).toBe("dark")
  })
})
