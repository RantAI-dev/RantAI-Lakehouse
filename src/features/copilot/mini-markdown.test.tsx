import { act, fireEvent, render, within } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import { MiniMarkdown, tableTsv } from "./mini-markdown"

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

  it("renders an unverified number inside bold as a marker, not as literal tag text", () => {
    const { container } = render(
      <MiniMarkdown text='**Total: <span data-unverified="true">13</span>**' />,
    )
    const strong = container.querySelector("strong")
    expect(strong).not.toBeNull()
    const marked = within(strong as HTMLElement).getByText("13")
    expect(marked.getAttribute("data-unverified")).toBe("true")
    expect(container.textContent).not.toContain("<span")
  })

  it("renders an unverified number inside italic as a marker, not as literal tag text", () => {
    const { container } = render(
      <MiniMarkdown text='*Total: <span data-unverified="true">13</span>*' />,
    )
    const em = container.querySelector("em")
    expect(em).not.toBeNull()
    const marked = within(em as HTMLElement).getByText("13")
    expect(marked.getAttribute("data-unverified")).toBe("true")
    expect(container.textContent).not.toContain("<span")
  })

  it("renders bold without a marker as plain strong text, unchanged", () => {
    const { container } = render(<MiniMarkdown text="**Rows: 42**" />)
    const strong = container.querySelector("strong")
    expect(strong?.textContent).toBe("Rows: 42")
    expect(strong?.children.length).toBe(0)
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

// A number the server marked must never reach the reader as tag text, in
// whatever Markdown context the model put it. Contexts that render inline
// tokens keep the mark; contexts that render the text verbatim (code) drop
// the tags and show the bare number.
const MARKED = '<span data-unverified="true">0,026</span>'

function expectNoMarkupText(container: HTMLElement) {
  expect(container.textContent).not.toContain("data-unverified=")
  expect(container.textContent).not.toContain("<span")
}

function expectMarkedNumber(container: HTMLElement) {
  const marked = container.querySelector("span[data-unverified]")
  expect(marked).not.toBeNull()
  expect(marked?.textContent).toBe("0,026")
}

describe("MiniMarkdown never shows a marked number as markup", () => {
  const markedContexts: [string, string][] = [
    ["plain text", `Rasio ${MARKED} per bulan.`],
    ["bold", `**Rasio ${MARKED}**`],
    ["italic", `*Rasio ${MARKED}*`],
    ["link text", `[rasio ${MARKED}](https://example.com/rasio)`],
    ["a list item", `- Rasio ${MARKED}`],
    ["a numbered list item", `1. Rasio ${MARKED}`],
    ["a quote", `> Rasio ${MARKED}`],
    ["a level-one heading", `# Rasio ${MARKED}`],
    ["a level-two heading", `## Rasio ${MARKED}`],
    ["a level-three heading", `### Rasio ${MARKED}`],
    ["a table cell", `| a | b |\n|---|---|\n| 1 | ${MARKED} |`],
    ["a table header cell", `| a | ${MARKED} |\n|---|---|\n| 1 | 2 |`],
  ]

  for (const [name, text] of markedContexts) {
    it(`keeps the mark on a marked number in ${name} and prints no tag text`, () => {
      const { container } = render(<MiniMarkdown text={text} />)
      expectNoMarkupText(container)
      expectMarkedNumber(container)
    })
  }

  it("keeps a marked number inside link text clickable, with the mark inside the anchor", () => {
    const { container } = render(
      <MiniMarkdown text={`[rasio ${MARKED}](https://example.com/rasio)`} />,
    )
    const anchor = container.querySelector("a")
    expect(anchor?.getAttribute("href")).toBe("https://example.com/rasio")
    expect(anchor?.querySelector("span[data-unverified]")?.textContent).toBe("0,026")
  })

  it("shows a marked number inside an inline code span as bare code text, without tag text", () => {
    const { container } = render(<MiniMarkdown text={`Nilai \`rasio ${MARKED}\` dihitung.`} />)
    expectNoMarkupText(container)
    expect(container.querySelector("code")?.textContent).toBe("rasio 0,026")
    expect(container.querySelector("span[data-unverified]")).toBeNull()
  })

  it("shows a marked number inside a fenced code block as bare code text, without tag text", () => {
    const { container } = render(
      <MiniMarkdown text={`\`\`\`sql\nSELECT ${MARKED} AS ratio;\n\`\`\``} />,
    )
    expectNoMarkupText(container)
    expect(container.querySelector("pre code")?.textContent).toBe("SELECT 0,026 AS ratio;")
  })

  it("copies a fenced code block without the marker tags", async () => {
    const copied: string[] = []
    const original = Object.getOwnPropertyDescriptor(navigator, "clipboard")
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: (t: string) => (copied.push(t), Promise.resolve()) },
    })
    try {
      const { container } = render(
        <MiniMarkdown text={`\`\`\`sql\nSELECT ${MARKED} AS ratio;\n\`\`\``} />,
      )
      await act(async () => {
        fireEvent.click(within(container).getByLabelText("Copy code"))
      })
    } finally {
      if (original) Object.defineProperty(navigator, "clipboard", original)
      else delete (navigator as { clipboard?: unknown }).clipboard
    }
    expect(copied).toEqual(["SELECT 0,026 AS ratio;"])
  })
})

describe("tableTsv", () => {
  it("copies a table as tab-separated plain values, dropping markdown and citation markers", () => {
    const tsv = tableTsv(
      ["Mart", "**Rows**"],
      [["`serving.mart_event`", '<span data-unverified="true">998</span>']],
    )
    expect(tsv).toBe("Mart\tRows\nserving.mart_event\t998")
  })
})
