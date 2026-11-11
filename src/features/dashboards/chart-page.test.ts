// `WEB-7` D6: the chart pages' addresses, and the assistant's draft handoff.
// Pure and DOM-only (sessionStorage is stubbed by the test), no network.

import { expect, test } from "bun:test"
import {
  CHART_DRAFT_KEY,
  builtInBoardEditable,
  chartEditHref,
  chartNewHref,
  chartParamValues,
  isBuiltInBoard,
  takeChartDraft,
  writeChartDraft,
} from "./chart-page"

test("the new and edit addresses name the board, the chart, and only the parameters that exist", () => {
  expect(chartNewHref("b1")).toBe("/dashboards/b1/charts/new")
  expect(chartEditHref("b1", "c1")).toBe("/dashboards/b1/charts/c1/edit")
  // A short parameter value rides in the query string; nothing else does.
  expect(chartNewHref("b1", { region: "eu", n: 3 })).toBe(
    `/dashboards/b1/charts/new?params=${encodeURIComponent('{"region":"eu","n":3}')}`
  )
  expect(chartEditHref("b1", "c1", {})).toBe("/dashboards/b1/charts/c1/edit")
  // A board or chart id that holds a slash stays one path segment.
  expect(chartNewHref("a/b")).toBe("/dashboards/a%2Fb/charts/new")
})

test("the parameter values are read back from the address, and a broken one opens the form anyway", () => {
  expect(chartParamValues("?params=%7B%22region%22%3A%22eu%22%7D")).toEqual({ region: "eu" })
  expect(chartParamValues("?params=" + encodeURIComponent('{"n":3,"odd":true}'))).toEqual({ n: 3 })
  expect(chartParamValues("")).toBeUndefined()
  expect(chartParamValues("?params=not-json")).toBeUndefined()
  // A list is not a value map.
  expect(chartParamValues(`?params=${encodeURIComponent("[1,2]")}`)).toBeUndefined()
})

test("a draft written for a board is taken once, and only by that board", () => {
  window.sessionStorage.clear()
  try {
    writeChartDraft({ board: "b1", def: { title: "Visitors", kind: "bar" } })
    expect(takeChartDraft("b2")).toBeNull()
    // Another board's page took it away rather than leaving it to surprise the next visit.
    expect(window.sessionStorage.getItem(CHART_DRAFT_KEY)).toBeNull()
    expect(takeChartDraft("b1")).toBeNull()

    writeChartDraft({ board: "b1", def: { title: "Visitors", kind: "bar" } })
    expect(takeChartDraft("b1")).toEqual({ board: "b1", def: { title: "Visitors", kind: "bar" } })
    // Read once: a reload of the page opens it empty.
    expect(takeChartDraft("b1")).toBeNull()
  } finally {
    window.sessionStorage.clear()
  }
})

test("a draft that cannot be stored leaves the page empty instead of throwing", () => {
  window.sessionStorage.clear()
  const original = globalThis.sessionStorage.setItem
  try {
    Object.defineProperty(window.sessionStorage, "setItem", {
      configurable: true,
      value: () => {
        throw new Error("blocked")
      },
    })
    writeChartDraft({ board: "b1", def: { title: "T" } })
    expect(takeChartDraft("b1")).toBeNull()
  } finally {
    Object.defineProperty(window.sessionStorage, "setItem", {
      configurable: true,
      value: original,
    })
    window.sessionStorage.clear()
  }
})

test("a draft the assistant made keeps the step it came from, so the chat can be told", () => {
  window.sessionStorage.clear()
  try {
    writeChartDraft({
      board: "b1",
      def: { title: "Visitors" },
      complete: { messageIndex: 2, stepIndex: 1 },
    })
    expect(takeChartDraft("b1")?.complete).toEqual({ messageIndex: 2, stepIndex: 1 })
    // A hand-edited address is not one: the page saves the chart without
    // touching a step it cannot name.
    writeChartDraft({
      board: "b1",
      def: { title: "Visitors" },
      complete: { messageIndex: -1, stepIndex: 1 } as never,
    })
    expect(takeChartDraft("b1")?.complete).toBeUndefined()
  } finally {
    window.sessionStorage.clear()
  }
})

test("the built-in dashboard is edited by the starting point the server reads at the root", () => {
  expect(isBuiltInBoard("default")).toBe(true)
  expect(isBuiltInBoard("")).toBe(true)
  expect(isBuiltInBoard("b1")).toBe(false)
  // GET /api/dashboard/access refuses `default`; the root level is the grant
  // (`identity:write` is manage everywhere, `dashboard:write` is manage).
  expect(builtInBoardEditable((p) => p === "dashboard:write")).toBe(true)
  expect(builtInBoardEditable((p) => p === "identity:write")).toBe(true)
  expect(builtInBoardEditable((p) => p === "dashboard:read")).toBe(false)
  expect(builtInBoardEditable(() => false)).toBe(false)
})
