"use client"

import * as React from "react"

import { CreateSheet } from "@/components/patterns/create-sheet"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { levelAtLeast } from "@/lib/access-level"
import { useService } from "@/hooks/use-service"
import { sqlSummary } from "@/lib/sql-text"
import { queryService } from "@/services"
import type {
  QueryFolder,
  QueryParamDef,
  SaveQueryInput,
  SavedQuery,
} from "@/services/contracts/queries"
import { FolderSharingNote } from "./folder-sharing-note"
import {
  PERSONAL_SPACE_LABEL,
  SHARED_SPACE_LABEL,
  buildSavedQueryTree,
  flattenTreeFolders,
} from "./saved-query-tree"

/**
 * What the picker sends for the caller's own space: no folder at all.
 *
 * The server reads the three answers apart: an absent folder is the
 * caller's own personal space, `null` is the shared space, and a folder's
 * id is that folder.
 */
const PERSONAL_SPACE = "personal"

/** What the picker sends for the shared space: the root of the tree. */
const SHARED_SPACE = "shared"

const NO_PARAMETERS: readonly QueryParamDef[] = []

/**
 * Name the SQL in the editor and keep it.
 *
 * A copy lands in the folder the picker names — the caller's own space by
 * default — and a change keeps the saved query where it is. A saved query
 * is not shared with the team by itself any more: where it lands is the
 * caller's to choose (`QS-3` C4).
 */
export function SaveQuerySheet({
  open,
  onOpenChange,
  sql,
  parameters = NO_PARAMETERS,
  saving,
  error,
  onSave,
  mode = "create",
  savedId = null,
  viewerId,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly sql: string
  /**
   * The definitions of the `{{name}}` parameters in `sql` (`QS-5`), with their
   * defaults. Never the values the tab holds: a saved query starts at its defaults.
   */
  readonly parameters?: readonly QueryParamDef[]
  readonly saving: boolean
  readonly error?: string | null
  readonly onSave: (input: SaveQueryInput) => Promise<unknown>
  /** "update" changes the saved query the tab came from; "create" makes a new one. */
  readonly mode?: "create" | "update"
  /** The saved query the tab came from, when it came from one. */
  readonly savedId?: string | null
  /** The signed-in person, who is left out of "Shared with"; `undefined` leaves nobody out. */
  readonly viewerId: string | undefined
}) {
  const [title, setTitle] = React.useState("")
  const [tags, setTags] = React.useState("")
  const [folder, setFolder] = React.useState(PERSONAL_SPACE)

  // The picker starts in the caller's own space every time the sheet opens:
  // where the copy before it landed is not where this one belongs.
  React.useEffect(() => {
    if (!open) return
    setFolder(PERSONAL_SPACE)
  }, [open])

  // The folders a copy may land in. Only a copy asks: a change keeps the
  // query where it is, which is the Move dialog's business.
  const folders = useService(
    (signal) =>
      open && mode === "create"
        ? queryService.listFolders(signal)
        : Promise.resolve([] as QueryFolder[]),
    [open, mode]
  )

  // The query the tab came from: its title and tags are what a copy starts
  // from, and what a change writes.
  const source = useService(
    (signal) =>
      open && savedId
        ? queryService.getSaved(savedId, signal)
        : Promise.resolve(null as SavedQuery | null),
    [open, savedId]
  )

  // Opening the sheet proposes a name taken from the query itself, so the
  // common case is "type nothing, press Save". A tab that came from a saved
  // query keeps that query's own name and tags instead.
  React.useEffect(() => {
    if (!open) return
    const found = source.status === "success" ? source.data : null
    setTitle(found ? found.title : sqlSummary(sql))
    setTags(found ? found.tags.join(", ") : "")
  }, [open, sql, source.status, source.data])

  const tree = React.useMemo(
    () => buildSavedQueryTree(folders.data ?? []),
    [folders.data]
  )
  const personalFolders = React.useMemo(
    () => (tree.personal ? flattenTreeFolders(tree.personal, PERSONAL_SPACE_LABEL) : []),
    [tree]
  )
  const sharedFolders = React.useMemo(
    () => flattenTreeFolders(tree.shared, ""),
    [tree]
  )

  /**
   * The folder the query lands in: the caller's own space sends no folder
   * at all, the shared space sends `null`, and a folder sends its id.
   */
  const folderId = React.useMemo(() => {
    if (mode !== "create") return undefined
    if (folder === SHARED_SPACE) return null
    if (folder === PERSONAL_SPACE) return undefined
    return folder
  }, [mode, folder])

  const noteFolderId =
    typeof folderId === "string" &&
    levelAtLeast(folders.data?.find((candidate) => candidate.id === folderId)?.level, "edit")
      ? folderId
      : null

  return (
    <CreateSheet
      open={open}
      onOpenChange={onOpenChange}
      title="Save query"
      description={
        mode === "update"
          ? "The change is written to the saved query, in the folder it already sits in."
          : "The query is kept in the folder you pick, with you recorded as the author."
      }
      submitLabel={mode === "update" ? "Save" : "Save as new"}
      submitting={saving}
      canSubmit={title.trim().length > 0 && sql.trim().length > 0}
      error={error}
      onSubmit={() => {
        const input: SaveQueryInput = {
          title: title.trim(),
          sql,
          tags: tags
            .split(",")
            .map((t) => t.trim())
            .filter(Boolean),
        }
        if (folderId !== undefined) input.folderId = folderId
        // A new query without placeholders sends no parameters at all. A
        // change always states the definitions it wants: a statement without
        // a placeholder wants none, so it sends an empty list and the server
        // clears the definitions with the SQL. What the saved query held
        // does not decide it, and the change does not wait for it to be read
        // (`#422 S1`).
        if (parameters.length > 0) input.parameters = [...parameters]
        else if (mode === "update") input.parameters = []
        void onSave(input).then((saved) => {
          if (saved) onOpenChange(false)
        })
      }}
    >
      <div className="space-y-2">
        <Label htmlFor="saved-query-title">Title</Label>
        <Input
          id="saved-query-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Revenue by region"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="saved-query-tags">Tags</Label>
        <Input
          id="saved-query-tags"
          value={tags}
          onChange={(e) => setTags(e.target.value)}
          placeholder="finance, gold"
        />
        <p className="text-xs text-muted-foreground">Separated by commas.</p>
      </div>
      {mode === "create" ? (
        <div className="space-y-2">
          <Label htmlFor="saved-query-folder">Folder</Label>
          <select
            id="saved-query-folder"
            value={folder}
            onChange={(event) => setFolder(event.target.value)}
            className="h-9 w-full min-w-0 rounded-lg border border-input bg-transparent px-2.5 py-1 text-sm transition-colors outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <optgroup label={PERSONAL_SPACE_LABEL}>
              <option value={PERSONAL_SPACE}>{PERSONAL_SPACE_LABEL}</option>
              {personalFolders.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </optgroup>
            <optgroup label={SHARED_SPACE_LABEL}>
              <option value={SHARED_SPACE}>{SHARED_SPACE_LABEL}</option>
              {sharedFolders.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </optgroup>
          </select>
          {/* Only a real folder can hold rules: the shared space and the
              caller's own space send `null` and `undefined`. The server
              refuses the access list to a caller below edit on the folder,
              so such a folder is not asked (QS-3 review). The key drops the
              previous folder's names at once; the hook keeps them until its
              effect resets (QS-3 review). */}
          <FolderSharingNote
            key={noteFolderId ?? ""}
            folderId={noteFolderId}
            viewerId={viewerId}
          />
        </div>
      ) : null}
    </CreateSheet>
  )
}
