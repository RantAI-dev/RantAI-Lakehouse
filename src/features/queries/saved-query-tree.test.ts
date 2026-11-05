// The saved-query tree the console draws (`QS-3` C1): the shared space and
// the caller's own space at the top, the folders under each, and the depth
// the server counts. The depth decides whether a folder may still hold a
// sub-folder (four levels), so a wrong depth is a wrong offer.

import { describe, expect, it } from "bun:test"
import type { QueryFolder } from "@/services/contracts/queries"
import {
  PERSONAL_SPACE_LABEL,
  SHARED_SPACE_LABEL,
  buildSavedQueryTree,
} from "./saved-query-tree"

function folder(over: Partial<QueryFolder> = {}): QueryFolder {
  return {
    id: "f1",
    parentId: null,
    name: "Reports",
    isPersonal: false,
    ownerId: null,
    level: "manage",
    ...over,
  }
}

const PERSONAL = folder({
  id: "p1",
  name: "Personal space of Rina Wijaya",
  isPersonal: true,
  ownerId: "u1",
  level: "manage",
})

describe("the saved-query tree", () => {
  it("puts the shared space and the caller's own space at the top", () => {
    const tree = buildSavedQueryTree([PERSONAL])

    expect(tree.shared.label).toBe(SHARED_SPACE_LABEL)
    expect(tree.shared.folder).toBe("root")
    expect(tree.personal?.label).toBe(PERSONAL_SPACE_LABEL)
    expect(tree.personal?.folder).toBe("p1")
    expect(tree.personal?.isPersonal).toBe(true)
  })

  it("has no personal space when the server sent none", () => {
    expect(buildSavedQueryTree([]).personal).toBeNull()
  })

  it("nests the folders under the space they sit in", () => {
    const tree = buildSavedQueryTree([
      PERSONAL,
      folder({ id: "shared1", parentId: null, name: "Team" }),
      folder({ id: "mine1", parentId: "p1", name: "Mine" }),
      folder({ id: "deep", parentId: "mine1", name: "Deeper" }),
    ])

    expect(tree.shared.children.map((c) => c.label)).toEqual(["Team"])
    expect(tree.personal?.children.map((c) => c.label)).toEqual(["Mine"])
    expect(tree.personal?.children[0]?.children.map((c) => c.label)).toEqual(["Deeper"])
  })

  it("counts the depth the server counts: the personal space is a level of its own", () => {
    const tree = buildSavedQueryTree([
      PERSONAL,
      folder({ id: "shared1", parentId: null }),
      folder({ id: "mine1", parentId: "p1" }),
      folder({ id: "mine2", parentId: "mine1" }),
    ])

    // A folder in the shared space is at depth 1; the personal space is a
    // folder of its own at depth 1, so a folder in it is at depth 2.
    expect(tree.shared.children[0]?.depth).toBe(1)
    expect(tree.personal?.depth).toBe(1)
    expect(tree.personal?.children[0]?.depth).toBe(2)
    expect(tree.personal?.children[0]?.children[0]?.depth).toBe(3)
  })

  it("shows a folder whose parent the caller may not see at the shared root", () => {
    const tree = buildSavedQueryTree([folder({ id: "orphan", parentId: "hidden" })])

    expect(tree.shared.children.map((c) => c.label)).toEqual(["Reports"])
  })
})
