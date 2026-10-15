import { describe, expect, test } from "bun:test"

import {
  daysLeft,
  daysLeftLabel,
  folderChoices,
  folderLabel,
  itemHref,
  labelledFolders,
  manageableFolders,
  restoreNote,
} from "./dashboard-organise"

const shared = { id: "f_1", name: "Sales", parentId: "" }
const mine = { id: "fp_me", name: "Personal space of Me", parentId: "", ownerId: "me" }
const theirs = { id: "fp_you", name: "Personal space of You", parentId: "", ownerId: "you" }

describe("personal space in the folder lists", () => {
  test("the caller's own space is called Personal space and another's keeps its name", () => {
    expect(folderLabel(mine, "me")).toBe("Personal space")
    expect(folderLabel(theirs, "me")).toBe("Personal space of You")
    expect(folderLabel(shared, "me")).toBe("Sales")
    expect(labelledFolders([shared, mine, theirs], "me").map((f) => f.name)).toEqual([
      "Sales",
      "Personal space",
      "Personal space of You",
    ])
  })

  test("a dashboard can be filed in an ordinary folder or the caller's own space only", () => {
    expect(folderChoices([shared, mine, theirs], "me").map((f) => f.id)).toEqual(["f_1", "fp_me"])
    expect(folderChoices([shared, mine, theirs], undefined).map((f) => f.id)).toEqual(["f_1"])
  })

  test("the folder manager never offers a personal space", () => {
    expect(manageableFolders([shared, mine, theirs]).map((f) => f.id)).toEqual(["f_1"])
  })
})

describe("trash entries", () => {
  const now = Date.parse("2026-10-14T00:00:00Z")

  test("days left rounds up and stops at zero", () => {
    expect(daysLeft("2026-10-17 00:00:00", now)).toBe(3)
    expect(daysLeft("2026-10-14 12:00:00", now)).toBe(1)
    expect(daysLeft("2026-10-01 00:00:00", now)).toBe(0)
    expect(daysLeft("not a date", now)).toBe(0)
    expect(daysLeft("2026-11-13 00:00:00", now)).toBe(30)
  })

  test("the label is singular, plural or due", () => {
    expect(daysLeftLabel("2026-10-15 00:00:00", now)).toBe("1 day left")
    expect(daysLeftLabel("2026-10-20 00:00:00", now)).toBe("6 days left")
    expect(daysLeftLabel("2026-10-14 00:00:00", now)).toBe("Due today")
  })

  test("a restored chart says where it will land", () => {
    expect(restoreNote({ kind: "board", boardState: "", boardName: "" })).toBe("With its charts")
    expect(restoreNote({ kind: "chart", boardState: "live", boardName: "Sales" })).toBe("Back on Sales")
    expect(restoreNote({ kind: "chart", boardState: "trash", boardName: "Sales" })).toBe(
      "Comes back with its dashboard",
    )
    expect(restoreNote({ kind: "chart", boardState: "gone", boardName: "" })).toBe(
      "Its dashboard is gone: on no dashboard",
    )
  })
})

describe("opening an item", () => {
  test("a dashboard opens itself and a chart opens its dashboard", () => {
    expect(itemHref({ kind: "board", id: "b_1" })).toBe("/dashboards/b_1")
    expect(itemHref({ kind: "chart", id: "u_1", board: "b_2" })).toBe("/dashboards/b_2")
    expect(itemHref({ kind: "chart", id: "u_1" })).toBe("/dashboards/default")
  })
})
