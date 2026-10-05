import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  appendId,
  DEFAULT_CARDS,
  DEFAULT_SHORTCUTS,
  hiddenIds,
  removeId,
  reorderAround,
  resolveLayout,
  sameLayout,
} from "./home-layout"

test("nothing saved resolves to the default cards, shortcuts and the last-opened preview", () => {
  const r = resolveLayout(null)
  assert.deepEqual(r.cards, ["dashboard-preview", "recent", "pipeline-runs"])
  assert.deepEqual(r.shortcuts, ["connect-source", "create-pipeline", "build-dashboard"])
  assert.equal(r.previewBoardId, null)
  assert.deepEqual(resolveLayout(undefined), r)
})

test("the defaults are copies, so changing a resolved layout cannot change them", () => {
  resolveLayout(null).cards.push("sources")
  assert.deepEqual([...DEFAULT_CARDS], ["dashboard-preview", "recent", "pipeline-runs"])
  assert.equal(DEFAULT_SHORTCUTS.length, 3)
})

test("ids this build does not know and repeats are dropped, order kept", () => {
  const r = resolveLayout({
    cards: ["sources", "retired-card", "recent", "sources", "Recent"],
    shortcuts: ["new-query", "gone", "new-query", "browse-catalog"],
    previewBoardId: "b1",
  })
  assert.deepEqual(r.cards, ["sources", "recent"])
  assert.deepEqual(r.shortcuts, ["new-query", "browse-catalog"])
  assert.equal(r.previewBoardId, "b1")
})

test("an explicitly empty list stays empty rather than falling back to the default", () => {
  const r = resolveLayout({ cards: [], shortcuts: [], previewBoardId: null })
  assert.deepEqual(r.cards, [])
  assert.deepEqual(r.shortcuts, [])
})

test("a list that is missing falls back to the default for that list only", () => {
  const r = resolveLayout({ cards: ["sources"] })
  assert.deepEqual(r.cards, ["sources"])
  assert.deepEqual(r.shortcuts, [...DEFAULT_SHORTCUTS])
})

test("a list of only unknown ids resolves to empty, not to the default", () => {
  assert.deepEqual(resolveLayout({ cards: ["x", "y"], shortcuts: [] }).cards, [])
})

test("shortcuts are capped at three, counting only known ones", () => {
  const r = resolveLayout({
    cards: [],
    shortcuts: ["nope", "connect-source", "create-pipeline", "build-dashboard", "new-query"],
  })
  assert.deepEqual(r.shortcuts, ["connect-source", "create-pipeline", "build-dashboard"])
})

test("a blank or non-string preview board id means the last opened", () => {
  assert.equal(resolveLayout({ cards: [], shortcuts: [], previewBoardId: "" }).previewBoardId, null)
  assert.equal(
    resolveLayout({ cards: [], shortcuts: [], previewBoardId: 3 as unknown as string })
      .previewBoardId,
    null,
  )
})

test("hiddenIds lists the catalogue entries that are not shown, in catalogue order", () => {
  assert.deepEqual(
    hiddenIds(
      ["dashboard-preview", "recent", "pipeline-runs", "sources", "open-alerts", "saved-queries"],
      ["recent", "sources"],
    ),
    ["dashboard-preview", "pipeline-runs", "open-alerts", "saved-queries"],
  )
})

test("removeId and appendId return new lists and respect the cap", () => {
  const ids = ["a", "b"] as const
  assert.deepEqual(removeId(ids, "a"), ["b"])
  assert.deepEqual(appendId(ids, "c"), ["a", "b", "c"])
  assert.deepEqual(appendId(ids, "a"), ["a", "b"])
  assert.deepEqual(appendId(ids, "c", 2), ["a", "b"])
  assert.deepEqual([...ids], ["a", "b"])
})

test("reorderAround keeps the pinned card where it was and reorders the rest", () => {
  assert.deepEqual(
    reorderAround(["recent", "dashboard-preview", "sources", "open-alerts"], ["open-alerts", "recent", "sources"], "dashboard-preview"),
    ["open-alerts", "dashboard-preview", "recent", "sources"],
  )
})

test("reorderAround with no pinned card present is a plain reorder", () => {
  assert.deepEqual(reorderAround(["a", "b", "c"], ["c", "a", "b"], "dashboard-preview"), ["c", "a", "b"])
})

test("reorderAround refuses an order that drops, repeats or invents a card", () => {
  const cards = ["a", "b", "c"]
  assert.deepEqual(reorderAround(cards, ["a", "b"], "p"), cards)
  assert.deepEqual(reorderAround(cards, ["a", "a", "b"], "p"), cards)
  assert.deepEqual(reorderAround(cards, ["a", "b", "z"], "p"), cards)
})

test("sameLayout compares order, lists and the preview board", () => {
  const a = resolveLayout(null)
  assert.equal(sameLayout(a, resolveLayout(null)), true)
  assert.equal(sameLayout(a, { ...a, cards: [...a.cards].reverse() }), false)
  assert.equal(sameLayout(a, { ...a, previewBoardId: "b" }), false)
  assert.equal(sameLayout(a, { ...a, shortcuts: [] }), false)
})
