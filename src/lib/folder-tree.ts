/**
 * Client-side view of the dashboard folder tree. The API returns folders as
 * a flat list (`GET /api/dashboard/folders`) and each board carries its
 * `folderId`; this builds the nested tree the switcher and the folder
 * dialog render. The server enforces the rules (`lakehouse_bi::folders`);
 * `MAX_FOLDER_DEPTH` here only decides whether to OFFER "New subfolder".
 */

/** Mirrors `lakehouse_bi::folders::MAX_FOLDER_DEPTH`. */
export const MAX_FOLDER_DEPTH = 4

export type FolderLike = { id: string; name: string; parentId: string }
export type BoardLike = { id: string; name: string; folderId?: string | null }

export type FolderNode<B extends BoardLike> = {
  folder: FolderLike
  /** 1 at the root. */
  depth: number
  children: FolderNode<B>[]
  boards: B[]
}

export type FolderTree<B extends BoardLike> = {
  /** Boards at the root, including any whose folder no longer exists. */
  rootBoards: B[]
  folders: FolderNode<B>[]
}

const byName = (a: { name: string }, b: { name: string }) =>
  a.name.localeCompare(b.name, undefined, { sensitivity: "base" })

export function buildFolderTree<B extends BoardLike>(
  folders: FolderLike[],
  boards: B[]
): FolderTree<B> {
  const known = new Set(folders.map((f) => f.id))
  const boardsIn = (id: string) => boards.filter((b) => (b.folderId ?? "") === id).sort(byName)
  const build = (parentId: string, depth: number, seen: Set<string>): FolderNode<B>[] =>
    folders
      .filter((f) => f.parentId === parentId && !seen.has(f.id))
      .sort(byName)
      .map((folder) => {
        const next = new Set(seen).add(folder.id)
        return {
          folder,
          depth,
          children: build(folder.id, depth + 1, next),
          boards: boardsIn(folder.id),
        }
      })
  return {
    // A board filed in a folder that was since deleted shows at the root
    // rather than disappearing from the list.
    rootBoards: boards
      .filter((b) => !b.folderId || !known.has(b.folderId))
      .sort(byName),
    folders: build("", 1, new Set()),
  }
}

/** Every folder in display order, with its depth — for pickers and lists. */
export function flattenFolders<B extends BoardLike>(
  nodes: FolderNode<B>[]
): { folder: FolderLike; depth: number }[] {
  return nodes.flatMap((n) => [{ folder: n.folder, depth: n.depth }, ...flattenFolders(n.children)])
}

/** "Sales / Q1 / Jan" for folder `id`; "" for the root or an unknown id. */
export function folderPath(folders: FolderLike[], id: string): string {
  const names: string[] = []
  const seen = new Set<string>()
  let current = folders.find((f) => f.id === id)
  while (current && !seen.has(current.id)) {
    seen.add(current.id)
    names.unshift(current.name)
    const parent = current.parentId
    current = parent ? folders.find((f) => f.id === parent) : undefined
  }
  return names.join(" / ")
}
