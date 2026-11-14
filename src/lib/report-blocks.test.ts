import { strict as assert } from "node:assert"
import { test } from "node:test"
import type { ReportBlock } from "@/services/contracts/dashboard-reports"
import {
  MAX_REPORT_CHARTS,
  blocksForSave,
  buildReportPageContext,
  canAddChart,
  chartCount,
  emptyBlock,
  insertBlock,
  listItems,
  listText,
  moveBlock,
  removeBlock,
  replaceBlock,
  safeReportUrl,
} from "./report-blocks"

const p = (text: string): ReportBlock => ({ type: "paragraph", text })
const c = (chart: string): ReportBlock => ({ type: "chart", chart })

test("safeReportUrl takes the web, email and console paths and nothing that runs code", () => {
  for (const ok of ["https://a.example", "HTTP://a.example", "mailto:a@b.example", "/dashboards/b_1"]) {
    assert.equal(safeReportUrl(ok), true, ok)
  }
  for (const bad of ["javascript:alert(1)", " JavaScript:x", "data:text/html,x", "//evil.example", "", "ftp://x"]) {
    assert.equal(safeReportUrl(bad), false, bad)
  }
})

test("canAddChart stops at the thirtieth chart", () => {
  const thirty = Array.from({ length: MAX_REPORT_CHARTS }, (_, i) => c(`c_${i}`))
  assert.equal(chartCount(thirty), 30)
  assert.equal(canAddChart(thirty.slice(1)), true)
  assert.equal(canAddChart([...thirty, p("text does not count")]), false)
})

test("insertBlock, moveBlock, removeBlock and replaceBlock keep the rest in order", () => {
  const blocks = [p("a"), p("b"), p("c")]
  assert.deepEqual(insertBlock(blocks, -1, p("x")).map((b) => (b as { text: string }).text), ["x", "a", "b", "c"])
  assert.deepEqual(insertBlock(blocks, 1, p("x")).map((b) => (b as { text: string }).text), ["a", "b", "x", "c"])
  assert.deepEqual(moveBlock(blocks, 2, -1).map((b) => (b as { text: string }).text), ["a", "c", "b"])
  assert.deepEqual(moveBlock(blocks, 0, -1), blocks, "the first block cannot move up")
  assert.deepEqual(moveBlock(blocks, 2, 1), blocks, "the last block cannot move down")
  assert.deepEqual(removeBlock(blocks, 1).map((b) => (b as { text: string }).text), ["a", "c"])
  assert.deepEqual(replaceBlock(blocks, 0, p("z"))[0], p("z"))
})

test("emptyBlock makes each text kind with nothing typed yet", () => {
  assert.deepEqual(emptyBlock("heading"), { type: "heading", text: "", level: 2 })
  assert.deepEqual(emptyBlock("list"), { type: "list", items: [""], ordered: false })
  assert.equal(emptyBlock("link").type, "link")
})

test("a list is edited as one item per line", () => {
  assert.equal(listText(["a", "b"]), "a\nb")
  assert.deepEqual(listItems("a\n\nb"), ["a", "", "b"])
})

test("blocksForSave drops blank list lines and the dashboard of a chart, and keeps text as typed", () => {
  const saved = blocksForSave([
    { type: "list", items: ["a", " ", "b"] },
    { type: "chart", chart: "c_1", board: "b_1" },
    p("<b>kept</b>"),
  ])
  assert.deepEqual(saved, [
    { type: "list", items: ["a", "b"] },
    { type: "chart", chart: "c_1" },
    p("<b>kept</b>"),
  ])
})

test("buildReportPageContext names the report's id and title for update_report", () => {
  const text = buildReportPageContext({ id: "r1", title: 'Q3 "visitors"', chartCount: 2 })
  assert.ok(text.includes("report id r1"), text)
  assert.ok(text.includes('"Q3 \\"visitors\\""'), text)
  assert.ok(text.includes("update_report with id r1"), text)
  assert.ok(text.includes("2 of 30 charts"), text)
  const long = buildReportPageContext({ id: "r2", title: "x".repeat(500), chartCount: 0 })
  assert.ok(!long.includes("x".repeat(201)), "the title is cut at 200 characters")
})
