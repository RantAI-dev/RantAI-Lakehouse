import { strict as assert } from "node:assert"
import { test } from "node:test"
import type { ReportBlock } from "@/services/contracts/dashboard-reports"
import { SLASH_ITEMS, blocksToDoc, docChartCount, docToBlocks, matchSlashItems, nearChartLimit, type DocNode } from "./report-doc"

const EVERY_KIND: ReportBlock[] = [
  { type: "heading", text: "Arrivals", level: 1 },
  { type: "heading", text: "By **month**", level: 2 },
  { type: "heading", text: "Old small", level: 3 },
  { type: "paragraph", text: "Visits **rose** in *June*, see [the method](https://a.example/m). 2 \\* 3" },
  { type: "paragraph", text: "Two\nlines" },
  { type: "list", items: ["North *up*", "South"] },
  { type: "list", items: ["one", "two"], ordered: true },
  { type: "quote", text: "A *quoted* line" },
  { type: "divider" },
  { type: "chart", chart: "c_1" },
  { type: "chart", chart: "c_2", width: "wide" },
]

test("blocks → document → blocks keeps every block kind as it was", () => {
  const doc = blocksToDoc("Q3 visitors", EVERY_KIND)
  assert.deepEqual(docToBlocks(doc), { title: "Q3 visitors", blocks: EVERY_KIND })
  assert.equal(docChartCount(doc), 2)
})

test("an old link block becomes a paragraph holding the link, and saves that way", () => {
  const doc = blocksToDoc("T", [
    { type: "link", text: "Method [v2]", url: " https://a.example/m " },
    { type: "link", text: "", url: "/dashboards/b_1" },
    { type: "link", text: "Run", url: "javascript:alert(1)" },
  ])
  assert.deepEqual(docToBlocks(doc).blocks, [
    { type: "paragraph", text: "[Method \\[v2\\]](https://a.example/m)" },
    { type: "paragraph", text: "[/dashboards/b_1](/dashboards/b_1)" },
    { type: "paragraph", text: "Run" },
  ])
})

test("an empty report is a title and one line to write on", () => {
  const doc = blocksToDoc("", [])
  assert.deepEqual(doc, { type: "doc", content: [{ type: "title" }, { type: "paragraph" }] })
  assert.deepEqual(docToBlocks(doc), { title: "", blocks: [] })
})

test("blank lines, empty lists and quotes, chartless charts and drafts are not saved", () => {
  const doc: DocNode = {
    type: "doc",
    content: [
      { type: "title", content: [{ type: "text", text: "  Spaced   title " }] },
      { type: "paragraph" },
      { type: "paragraph", content: [{ type: "text", text: "   " }] },
      { type: "bulletList", content: [{ type: "listItem", content: [{ type: "paragraph" }] }] },
      { type: "blockquote", content: [{ type: "paragraph" }] },
      { type: "chart", attrs: { chart: null, width: "text" } },
      { type: "aiDraft", attrs: { text: "not yet" } },
      { type: "heading", attrs: { level: 2 } },
      { type: "paragraph", content: [{ type: "text", text: "kept" }] },
    ],
  }
  assert.deepEqual(docToBlocks(doc), { title: "Spaced title", blocks: [{ type: "paragraph", text: "kept" }] })
})

test("an indented list item is the next item of the same list", () => {
  const item = (text: string, nested?: DocNode): DocNode => ({
    type: "listItem",
    content: [{ type: "paragraph", content: [{ type: "text", text }] }, ...(nested ? [nested] : [])],
  })
  const doc: DocNode = {
    type: "doc",
    content: [
      { type: "title" },
      { type: "bulletList", content: [item("a", { type: "bulletList", content: [item("a1"), item("a2")] }), item("b")] },
    ],
  }
  assert.deepEqual(docToBlocks(doc).blocks, [{ type: "list", items: ["a", "a1", "a2", "b"] }])
})

test("typed characters that mean marks are escaped on save", () => {
  const doc: DocNode = {
    type: "doc",
    content: [
      { type: "title", content: [{ type: "text", text: "**Title** stays plain" }] },
      { type: "paragraph", content: [{ type: "text", text: "2*3 [x](javascript:y)" }] },
    ],
  }
  assert.deepEqual(docToBlocks(doc), {
    title: "**Title** stays plain",
    blocks: [{ type: "paragraph", text: "2\\*3 \\[x\\]\\(javascript:y\\)" }],
  })
})

test("the / menu finds items by label or keyword, in menu order", () => {
  assert.deepEqual(matchSlashItems(SLASH_ITEMS, "").map((i) => i.id), SLASH_ITEMS.map((i) => i.id))
  assert.deepEqual(matchSlashItems(SLASH_ITEMS, "head").map((i) => i.id), ["h1", "h2"])
  assert.deepEqual(matchSlashItems(SLASH_ITEMS, "LIST").map((i) => i.id), ["bullet", "numbered"])
  assert.deepEqual(matchSlashItems(SLASH_ITEMS, "kpi").map((i) => i.id), ["chart"])
  assert.deepEqual(matchSlashItems(SLASH_ITEMS, "hr").map((i) => i.id), ["divider"])
  assert.deepEqual(matchSlashItems(SLASH_ITEMS, "zzz"), [])
})

test("the chart count shows from 25 of 30", () => {
  assert.equal(nearChartLimit(24), false)
  assert.equal(nearChartLimit(25), true)
  assert.equal(nearChartLimit(30), true)
})
