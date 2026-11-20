import { strict as assert } from "node:assert"
import { test } from "node:test"
import { focalBoard } from "./home-focal"

const boards = [
  { id: "old", name: "Old", chartCount: 2, updatedAt: "2026-11-01T00:00:00Z" },
  { id: "new", name: "New", chartCount: 3, updatedAt: "2026-11-13T00:00:00Z" },
  { id: "empty", name: "Empty", chartCount: 0, updatedAt: "2026-11-14T00:00:00Z" },
  { id: "fav", name: "Fav", chartCount: 1, updatedAt: "2026-10-01T00:00:00Z" },
]

const pick = (o: Partial<Parameters<typeof focalBoard>[0]>) =>
  focalBoard({ boards, chosenId: null, favouriteIds: [], lastOpenedId: null, ...o })

test("the board chosen in Customize wins", () => {
  assert.deepEqual(pick({ chosenId: "old", favouriteIds: ["fav"], lastOpenedId: "new" }), {
    board: boards[0],
    reason: "chosen",
  })
})

test("with nothing chosen the first favourite that has charts comes next", () => {
  const p = pick({ favouriteIds: ["empty", "fav", "old"], lastOpenedId: "new" })
  assert.equal(p?.board.id, "fav")
  assert.equal(p?.reason, "favourite")
})

test("a chosen board that was deleted or emptied falls through, never to an empty widget", () => {
  assert.equal(pick({ chosenId: "gone", favouriteIds: ["fav"] })?.board.id, "fav")
  assert.equal(pick({ chosenId: "empty", lastOpenedId: "old" })?.board.id, "old")
})

test("without favourites the board last opened here is shown, then the most recently updated", () => {
  assert.equal(pick({ lastOpenedId: "old" })?.reason, "last-opened")
  const p = pick({ lastOpenedId: "empty" })
  assert.equal(p?.board.id, "new")
  assert.equal(p?.reason, "recent")
})

test("no board with charts means no focal board", () => {
  assert.equal(focalBoard({ boards: [boards[2]], chosenId: null, favouriteIds: [], lastOpenedId: null }), null)
  assert.equal(focalBoard({ boards: [], chosenId: "x", favouriteIds: ["y"], lastOpenedId: "z" }), null)
})
