/**
 * The saved-query tree the console draws (`QS-3` C1).
 *
 * The server sends the tree as a flat list (`GET /api/query/folders`): the
 * shared space has no node of its own, so a folder with no parent sits in
 * it, and every person has one personal space — a folder of its own — that
 * only that person sees. This builds the two spaces and the folders under
 * each, and carries the depth the server counts: a folder in the shared
 * space is at depth 1 and the personal space is a folder at depth 1, so a
 * folder in it is at depth 2. The depth decides whether a folder may still
 * hold a sub-folder (four levels).
 */

import type { QueryFolder } from "@/services/contracts/queries"

/** What the caller's own space is called in the tree (C1). */
export const PERSONAL_SPACE_LABEL = "Personal space (only you)"

/** What the shared space is called in the tree (C1). */
export const SHARED_SPACE_LABEL = "Shared"

/** The `folder` value the shared space takes: the server's own word for it. */
export const SHARED_SPACE_FOLDER = "root"

export type SavedQueryTreeNode = {
  /** The node's id: the folder's id, or `"shared"` for the shared space. */
  id: string
  /** What the node shows. */
  label: string
  /** The name the server sent, shown as the node's tooltip. */
  name: string
  /** The `folder` value the list call takes for this node. */
  folder: string
  /** The folder the node stands for; the shared space has none. */
  record: QueryFolder | null
  /** Whether the node is the caller's own personal space. */
  isPersonal: boolean
  /**
   * The level the caller holds on the node; `""` for the shared space,
   * which has no node of its own and therefore no level.
   */
  level: string
  /**
   * The depth the server counts: 1 for a folder in the shared space, 1 for
   * the personal space (it is a folder of its own) and 2 for a folder in
   * it. The shared space is 0 — it is not a folder.
   */
  depth: number
  children: SavedQueryTreeNode[]
}

export type SavedQueryTree = {
  /** The caller's own space, when the server sent one. */
  personal: SavedQueryTreeNode | null
  /** The shared space: the root of the tree. */
  shared: SavedQueryTreeNode
}

/**
 * The tree of the folders the caller may see.
 *
 * A folder whose parent is not in the list shows at the shared root: the
 * server drops a parent the caller may not see, and the shared space is
 * the only place a folder with no visible parent can be.
 */
export function buildSavedQueryTree(folders: readonly QueryFolder[]): SavedQueryTree {
  const personalFolder = folders.find((folder) => folder.isPersonal) ?? null
  const personalId = personalFolder?.id ?? null
  const known = new Set(folders.map((folder) => folder.id))

  const childrenOf = new Map<string | null, QueryFolder[]>()
  for (const folder of folders) {
    if (folder.id === personalId) continue
    const parentId =
      folder.parentId === null
        ? null
        : folder.parentId === personalId || known.has(folder.parentId)
          ? folder.parentId
          : null
    const bucket = childrenOf.get(parentId) ?? []
    bucket.push(folder)
    childrenOf.set(parentId, bucket)
  }

  const build = (parentId: string | null, depth: number, seen: Set<string>): SavedQueryTreeNode[] =>
    (childrenOf.get(parentId) ?? [])
      .filter((folder) => !seen.has(folder.id))
      .map((folder) => {
        const next = new Set(seen).add(folder.id)
        return {
          id: folder.id,
          label: folder.name,
          name: folder.name,
          folder: folder.id,
          record: folder,
          isPersonal: false,
          level: folder.level,
          depth,
          children: build(folder.id, depth + 1, next),
        }
      })

  const shared: SavedQueryTreeNode = {
    id: "shared",
    label: SHARED_SPACE_LABEL,
    name: SHARED_SPACE_LABEL,
    folder: SHARED_SPACE_FOLDER,
    record: null,
    isPersonal: false,
    level: "",
    depth: 0,
    children: build(null, 1, new Set()),
  }

  const personal: SavedQueryTreeNode | null = personalFolder
    ? {
        id: personalFolder.id,
        label: PERSONAL_SPACE_LABEL,
        name: personalFolder.name,
        folder: personalFolder.id,
        record: personalFolder,
        isPersonal: true,
        level: personalFolder.level,
        depth: 1,
        children: build(personalFolder.id, 2, new Set([personalFolder.id])),
      }
    : null

  return { personal, shared }
}
