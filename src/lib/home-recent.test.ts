import { strict as assert } from "node:assert"
import { test } from "node:test"
import { recentItems, type RecentSource } from "./home-recent"

const it = (id: string, at?: string): RecentSource => ({ id, title: id, href: `/${id}`, at })

test("the three kinds are merged newest first", () => {
  const out = recentItems(
    {
      dashboard: [it("board", "2026-10-01T00:00:00Z")],
      conversation: [it("chat", "2026-10-02T05:00:00Z")],
      query: [it("query", "2026-09-30T00:00:00Z")],
    },
    null,
  )
  assert.deepEqual(out.map((r) => [r.kind, r.id]), [["conversation", "chat"], ["dashboard", "board"], ["query", "query"]])
})

test("the dashboard last opened leads even when it is the oldest", () => {
  const out = recentItems(
    {
      dashboard: [it("new", "2026-10-02T00:00:00Z"), it("old", "2026-09-01T00:00:00Z")],
      conversation: [it("chat", "2026-10-02T05:00:00Z")],
      query: [],
    },
    "old",
  )
  assert.deepEqual(out.map((r) => r.id), ["old", "chat", "new"])
  assert.equal(out[0].lastOpened, true)
  assert.equal(out[2].lastOpened, false)
})

test("a remembered id only matches a dashboard, never another kind with that id", () => {
  const out = recentItems({ dashboard: [], conversation: [it("x", "2026-10-01T00:00:00Z")], query: [] }, "x")
  assert.equal(out[0].lastOpened, false)
})

test("one busy kind cannot crowd out the others, and the total is capped", () => {
  const chats = Array.from({ length: 9 }, (_, i) => it(`c${i}`, `2026-10-02T0${i}:00:00Z`))
  const out = recentItems(
    { dashboard: [it("b", "2026-09-01T00:00:00Z")], conversation: chats, query: [it("q", "2026-09-02T00:00:00Z")] },
    null,
  )
  assert.deepEqual(out.map((r) => r.id), ["c8", "c7", "c6", "q", "b"])
  const five = Array.from({ length: 5 }, (_, i) => it(`${i}`, `2026-10-0${i + 1}T00:00:00Z`))
  assert.equal(recentItems({ dashboard: five, conversation: five, query: five }, null, 7, 3).length, 7)
})

test("a missing or unparseable time sorts last instead of throwing", () => {
  const out = recentItems(
    { dashboard: [it("none"), it("bad", "not a date")], conversation: [it("dated", "2026-10-01T00:00:00Z")], query: [] },
    null,
  )
  assert.equal(out[0].id, "dated")
  assert.equal(out.length, 3)
})
