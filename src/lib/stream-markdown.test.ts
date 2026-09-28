import { describe, expect, it } from "bun:test"

import { settleStreamingMarkdown } from "./stream-markdown"

describe("settleStreamingMarkdown", () => {
  it("closes bold that the stream has opened but not yet closed", () => {
    expect(settleStreamingMarkdown("Total is **5")).toBe("Total is **5**")
  })

  it("closes inline code that is still being written", () => {
    expect(settleStreamingMarkdown("Use `serving.mart")).toBe("Use `serving.mart`")
  })

  it("holds back a table until its delimiter row arrives", () => {
    expect(settleStreamingMarkdown("Rows:\n\n| region | total |")).toBe("Rows:\n")
    const ready = "Rows:\n\n| region | total |\n|---|---|\n| Jakarta | 5 |"
    expect(settleStreamingMarkdown(ready)).toBe(ready)
  })

  it("does not let a lone list marker turn the paragraph above into a heading", () => {
    expect(settleStreamingMarkdown("Here is the list:\n-")).toBe("Here is the list:\n")
  })

  it("shows a half-typed link as its text", () => {
    expect(settleStreamingMarkdown("See [the docs](https://exa")).toBe("See the docs")
  })

  it("leaves backticks inside a fenced block alone", () => {
    const text = "```sql\nSELECT `a`"
    expect(settleStreamingMarkdown(text)).toBe(text)
  })

  it("leaves finished markdown unchanged", () => {
    const text = "**Done.** Use `mart_x`.\n\n- one\n- two"
    expect(settleStreamingMarkdown(text)).toBe(text)
  })
})
