import { render, within } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import { MiniMarkdown } from "./mini-markdown"

// WS7 item F6: a reader must be able to see WHY an unverified number is
// marked, and an omitted table must read as deliberate product behaviour,
// not a crash. This mounts real backend output shapes (the exact strings
// citations.rs's annotate_answer emits) through happy-dom.
//
// Queries are scoped to each render's own `container` via `within` — the
// test process shares one global `document` across every test file
// (happydom.ts registers it once), and other suites render fixtures with
// generic text like "1" that a document-wide `screen` query would collide
// with.
describe("MiniMarkdown citation rendering (WS7 item F6)", () => {
  it("renders an unverified number with a title explaining the check, not just a bare underline", () => {
    const { container } = render(
      <MiniMarkdown text='Total pendapatan adalah <span data-unverified="true">999999</span>.' />,
    )
    const marked = within(container).getByText("999999")
    expect(marked.getAttribute("title")).toBe("Could not be matched to any tool result")
    expect(marked.getAttribute("data-unverified")).toBe("true")
  })

  it("leaves a verified number as plain text with no marker at all", () => {
    const { container } = render(<MiniMarkdown text="Ada 42 baris." />)
    expect(within(container).getByText("Ada 42 baris.")).toBeDefined()
    expect(container.querySelector("[data-unverified]")).toBeNull()
  })

  it("renders the omitted-table note as prose, not as a table or an error", () => {
    const { container } = render(
      <MiniMarkdown text="Berikut datanya:\n\n[table omitted: not backed by a tool result]\n\nSemoga membantu." />,
    )
    expect(
      within(container).getByText("[table omitted: not backed by a tool result]"),
    ).toBeDefined()
    expect(container.querySelector("table")).toBeNull()
  })

  it("wraps only the unverified cell in a table, keeping the grid intact", () => {
    const text = '| a | b |\n|---|---|\n| 1 | <span data-unverified="true">998</span> |'
    const { container } = render(<MiniMarkdown text={text} />)
    expect(within(container).getByText("1")).toBeDefined()
    const marked = within(container).getByText("998")
    expect(marked.getAttribute("data-unverified")).toBe("true")
    expect(container.querySelector("table")).not.toBeNull()
  })
})
