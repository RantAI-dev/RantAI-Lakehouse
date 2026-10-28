/**
 * Pure rules of the folders on the Uploaded files tab (`DATA-35`), kept here so
 * they are testable without a DOM. The API is the authority on every rule
 * (`lakehouse-store/src/upload_folders.rs`); these exist so the screen offers only
 * what it will accept and says why a control is off, in the API's own words.
 *
 * The tree itself is `@/lib/folder-tree`, shared with the dashboards. Its
 * `MAX_FOLDER_DEPTH` (4) is the dashboards' rule and is only read by the dashboard
 * dialogs, so this module has its own number and the shared file is untouched.
 */

import { buildFolderTree, flattenFolders, folderPath, type FolderLike } from "@/lib/folder-tree"
import type { Upload, UploadFolder } from "@/services/contracts/uploads"

/** Mirrors `upload_folders::MAX_DEPTH` (`DATA-35` decision 3): three levels, the first being the top level. */
export const UPLOAD_FOLDER_MAX_DEPTH = 3

/** The `folderId` filter and the tree entry for the files in no folder. */
export const NO_FOLDER = "none"

/** The tree entry for every file, whatever its folder. */
export const ALL_FILES = "all"

/**
 * The sentence next to a Delete that is off. It says what to do, as the API's own 409
 * does (`NOT_EMPTY` in `routes/uploads/folders.rs`); the counts come from the server.
 */
export function deleteFolderReason(folder: Pick<UploadFolder, "fileCount" | "childCount">): string | null {
  const parts: string[] = []
  if (folder.fileCount > 0) parts.push(`${folder.fileCount} ${folder.fileCount === 1 ? "file" : "files"}`)
  if (folder.childCount > 0) parts.push(`${folder.childCount} ${folder.childCount === 1 ? "folder" : "folders"}`)
  if (parts.length === 0) return null
  return `This folder holds ${parts.join(" and ")}. Move them out and delete its folders first.`
}

/** The shared tree's input: the API's `parentId: null` is the tree's top level, `""`. */
export function asFolderLikes(folders: readonly UploadFolder[]): FolderLike[] {
  return folders.map((f) => ({ id: f.id, name: f.name, parentId: f.parentId ?? "" }))
}

/** 1 for a top-level folder; 0 for an unknown id. */
export function folderDepth(folders: readonly UploadFolder[], id: string): number {
  return walk(asFolderLikes(folders), id)
}

function walk(folders: FolderLike[], id: string): number {
  let depth = 0
  const seen = new Set<string>()
  let current = folders.find((f) => f.id === id)
  while (current && !seen.has(current.id)) {
    seen.add(current.id)
    depth += 1
    const parent: string = current.parentId
    current = parent ? folders.find((f) => f.id === parent) : undefined
  }
  return depth
}

/** Whether a folder can have a folder made in it: the API refuses a fourth level. */
export function canNestIn(folders: readonly UploadFolder[], id: string): boolean {
  return folderDepth(folders, id) < UPLOAD_FOLDER_MAX_DEPTH
}

/** "Sales / Q1" for a folder, "No folder" for a file in none (or in one the list does not hold). */
export function folderLabel(folders: readonly UploadFolder[], id: string | undefined): string {
  if (id === undefined) return "No folder"
  const path = folderPath(asFolderLikes(folders), id)
  return path === "" ? "No folder" : path
}

/** Every folder in display order with its full path and depth, for a picker. */
export function folderChoices(folders: readonly UploadFolder[]): { id: string; path: string; depth: number }[] {
  const like = asFolderLikes(folders)
  return flattenFolders(buildFolderTree(like, []).folders).map(({ folder, depth }) => ({
    id: folder.id,
    path: folderPath(like, folder.id),
    depth,
  }))
}

/**
 * The folder the Table step offers for a table that exists (`DATA-35` decision 2):
 * where the newest upload loaded into `table` sits, or `undefined` when none is
 * listed or it sits in no folder. `uploads` is newest first, as the API lists it.
 */
export function folderOfTable(table: string, uploads: readonly Upload[]): string | undefined {
  return uploads.find((upload) => upload.bronzeTable === table)?.folderId
}

/** The folder every one of `uploads` sits in, or `undefined` when they differ or sit in none. */
export function commonFolder(uploads: readonly Upload[]): string | undefined {
  const first = uploads[0]?.folderId
  return first !== undefined && uploads.every((u) => u.folderId === first) ? first : undefined
}
