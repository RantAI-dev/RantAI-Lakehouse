import { describe, expect, it } from "bun:test"

import type { ReportBlock } from "@/services/contracts/dashboard-reports"
import { blocksToDoc, docToBlocks } from "./report-doc"
import { blocksToMarkdown, chartLine, markdownToBlocks } from "./report-markdown"

// BI-62 PR B (T8, D8): an assistant's draft becomes the report's blocks on
// Accept, and the report is sent to the assistant as Markdown.

describe("markdownToBlocks", () => {
  it("reads every kind the editor holds", () => {
    const md = [
      "# Arrivals",
      "",
      "## By month",
      "",
      "Visits **rose** in *June*,",
      "see [the method](https://a.example/m).",
      "",
      "- North",
      "- South *up*",
      "",
      "1. One",
      "2. Two",
      "",
      "> A quoted",
      "> line",
      "",
      "---",
      "",
      "The end.",
    ].join("\n")
    expect(markdownToBlocks(md)).toEqual([
      { type: "heading", text: "Arrivals", level: 1 },
      { type: "heading", text: "By month", level: 2 },
      { type: "paragraph", text: "Visits **rose** in *June*, see [the method](https://a.example/m)." },
      { type: "list", items: ["North", "South *up*"] },
      { type: "list", items: ["One", "Two"], ordered: true },
      { type: "quote", text: "A quoted\nline" },
      { type: "divider" },
      { type: "paragraph", text: "The end." },
    ])
  })

  it("reads each list and rule marker, and a deeper heading as level 3", () => {
    expect(markdownToBlocks("* a\n+ b\n- c")).toEqual([{ type: "list", items: ["a", "b", "c"] }])
    expect(markdownToBlocks("1) a\n2) b")).toEqual([{ type: "list", items: ["a", "b"], ordered: true }])
    expect(markdownToBlocks("***\n\n___\n\n- - -")).toEqual([{ type: "divider" }, { type: "divider" }, { type: "divider" }])
    expect(markdownToBlocks("### Small\n\n###### Smaller")).toEqual([
      { type: "heading", text: "Small", level: 3 },
      { type: "heading", text: "Smaller", level: 3 },
    ])
    // Closing hashes go; a hash inside a word stays.
    expect(markdownToBlocks("## Title ##\n\n# C#")).toEqual([
      { type: "heading", text: "Title", level: 2 },
      { type: "heading", text: "C#", level: 1 },
    ])
  })

  it("keeps an indented item in the same list and an indented line in its item", () => {
    expect(markdownToBlocks("- North\n  - Coast\n  more words\n- South")).toEqual([
      { type: "list", items: ["North", "Coast more words", "South"] },
    ])
    // Another marker without indentation starts another list.
    expect(markdownToBlocks("- a\n1. b")).toEqual([
      { type: "list", items: ["a"] },
      { type: "list", items: ["b"], ordered: true },
    ])
  })

  it("turns what it does not know into paragraph text, never dropping it", () => {
    expect(markdownToBlocks("| a | b |\n| - | - |\n| 1 | 2 |")).toEqual([
      { type: "paragraph", text: "| a | b | | - | - | | 1 | 2 |" },
    ])
    expect(markdownToBlocks("#nospace and `code`")).toEqual([{ type: "paragraph", text: "#nospace and `code`" }])
    // A fenced block is shown as written: its stars are escaped, not marks.
    expect(markdownToBlocks("```\n**x** [y](z)\nnext\n```")).toEqual([{ type: "paragraph", text: "\\*\\*x\\*\\* \\[y\\]\\(z\\)\nnext" }])
  })

  it("reads an answer wrapped whole in a markdown fence as the Markdown inside", () => {
    expect(markdownToBlocks("```markdown\n## Hi\n\nText\n```")).toEqual([
      { type: "heading", text: "Hi", level: 2 },
      { type: "paragraph", text: "Text" },
    ])
  })

  it("keeps an unsafe link's words and drops its address", () => {
    // The address ends at the first ")" (the subset's rule), so the second one is text.
    expect(markdownToBlocks("Click [here](javascript:alert(1)) now")).toEqual([{ type: "paragraph", text: "Click here\\) now" }])
    expect(markdownToBlocks("[x](data:text/html,<b>)")).toEqual([{ type: "paragraph", text: "x" }])
    expect(markdownToBlocks("A [console page](/dashboards/b_1).")).toEqual([{ type: "paragraph", text: "A [console page](/dashboards/b_1)." }])
  })

  it("leaves hostile text as text: markup, unbalanced stars, backslashes", () => {
    expect(markdownToBlocks("<script>alert(1)</script> **open")).toEqual([{ type: "paragraph", text: "<script>alert\\(1\\)</script> \\*\\*open" }])
    expect(markdownToBlocks("2 * 3 = 6 \\ and \\*")).toEqual([{ type: "paragraph", text: "2 \\* 3 = 6 \\\\ and \\*" }])
    // A link starts at the first "[" and its words never hold another link.
    expect(markdownToBlocks("[a [b](https://x.example)")).toEqual([{ type: "paragraph", text: "[a \\[b](https://x.example)" }])
    // Long hostile input is read in one pass and stays text.
    const many = "[".repeat(10_000)
    expect(markdownToBlocks(many)).toEqual([{ type: "paragraph", text: "\\[".repeat(10_000) }])
  })

  it("answers nothing for blank text, and CRLF reads like LF", () => {
    expect(markdownToBlocks("")).toEqual([])
    expect(markdownToBlocks("  \n\n ")).toEqual([])
    expect(markdownToBlocks("# A\r\n\r\nB")).toEqual([
      { type: "heading", text: "A", level: 1 },
      { type: "paragraph", text: "B" },
    ])
  })

  it("makes blocks the editor shows and saves unchanged", () => {
    const blocks = markdownToBlocks("# T\n\nA **b** *c* [d](https://e.example)\n\n- f\n\n> g\n\n---")
    expect(docToBlocks(blocksToDoc("Title", blocks)).blocks).toEqual(blocks)
  })
})

describe("blocksToMarkdown", () => {
  const labels: Record<string, { title: string; dashboard: string }> = { c_1: { title: "Visits by month", dashboard: "Sales" } }
  const label = (id: string) => labels[id]

  it("writes the report as Markdown, charts as one line each", () => {
    const blocks: ReportBlock[] = [
      { type: "heading", text: "Arrivals", level: 1 },
      { type: "heading", text: "Old small", level: 3 },
      { type: "paragraph", text: "Visits **rose**." },
      { type: "list", items: ["North", "South"] },
      { type: "list", items: ["one", "two"], ordered: true },
      { type: "quote", text: "A\nB" },
      { type: "divider" },
      { type: "link", text: "Method", url: "https://a.example/m" },
      { type: "link", text: "Run", url: "javascript:alert(1)" },
      { type: "chart", chart: "c_1", width: "wide" },
      { type: "chart", chart: "c_gone" },
    ]
    expect(blocksToMarkdown(blocks, label)).toBe(
      [
        "# Arrivals",
        "### Old small",
        "Visits **rose**.",
        "- North\n- South",
        "1. one\n2. two",
        "> A\n> B",
        "---",
        "[Method](https://a.example/m)",
        "Run",
        "[chart: Visits by month · Sales]",
        "[chart: not available]",
      ].join("\n\n"),
    )
  })

  it("reads back as the same blocks, charts aside", () => {
    const blocks: ReportBlock[] = [
      { type: "heading", text: "Arrivals", level: 1 },
      { type: "paragraph", text: "Visits **rose** in *June*." },
      { type: "list", items: ["North", "South"] },
      { type: "quote", text: "A\nB" },
      { type: "divider" },
    ]
    expect(markdownToBlocks(blocksToMarkdown(blocks, label))).toEqual(blocks)
  })

  it("names a chart without a dashboard by its title alone", () => {
    expect(chartLine({ title: "  A\n b ", dashboard: "" })).toBe("[chart: A b]")
    expect(chartLine({ title: "", dashboard: "Ops" })).toBe("[chart: Untitled chart · Ops]")
  })
})
