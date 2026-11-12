"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"

import { FormPage } from "@/components/patterns/form-page"
import { EmptyState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SelectField } from "@/components/patterns/select-field"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { levelAtLeast } from "@/lib/access-level"
import { rememberSavedTab } from "@/lib/query-draft"
import { sqlSummary } from "@/lib/sql-text"
import { queryService } from "@/services"
import type { QueryFolder, SaveQueryInput, SavedQuery } from "@/services/contracts/queries"
import { FolderSharingNote } from "./folder-sharing-note"
import { clearSaveQueryDraft, readSaveQueryDraft, type SaveQueryDraft } from "./save-draft"
import { savedQueryRequest } from "./saved-query-write"
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

/**
 * `/query-studio/save` (`WEB-7` T5, D2/D5): name the SQL in the editor and
 * keep it, as a page instead of a sheet over the editor.
 *
 * The editor writes what it would save to `sessionStorage` before it
 * navigates here (`save-draft.ts`), and this page reads it once. Opened any
 * other way it has no statement to save, so it says to open it from Query
 * Studio rather than showing an empty form. A successful save clears the
 * draft and goes back to the editor, whose own draft keeps the SQL.
 */
export function SaveQueryPage() {
  const [draft, setDraft] = React.useState<SaveQueryDraft | null | undefined>(undefined)

  React.useEffect(() => {
    // Read after mount, never during render: `sessionStorage` is absent on
    // the server, and reading it in render would make the first paint differ
    // from the markup Next sent.
    setDraft(readSaveQueryDraft())
  }, [])

  if (draft === undefined) return <LoadingSkeleton rows={4} />
  if (draft === null) return <NoDraft />
  return <SaveQueryForm draft={draft} />
}

/** The page without a draft: it was opened from somewhere else. */
function NoDraft() {
  return (
    <EmptyState
      title="Open this from Query Studio"
      description="Saving names the statement the editor holds, so the form starts there. Write a query, then choose Save."
      action={
        <Button variant="outline" size="sm" render={<Link href="/query-studio" />}>
          Open Query Studio
        </Button>
      }
    />
  )
}

/**
 * The form. A copy lands in the folder the picker names — the caller's own
 * space by default — and a change keeps the saved query where it is. A
 * saved query is not shared with the team by itself: where it lands is the
 * caller's to choose (`QS-3` C4).
 */
function SaveQueryForm({ draft }: { draft: SaveQueryDraft }) {
  const router = useRouter()
  const backHref = "/query-studio"
  const { user } = useAuth()
  const [title, setTitle] = React.useState("")
  const [tags, setTags] = React.useState("")
  const [folder, setFolder] = React.useState(PERSONAL_SPACE)
  const isChange = draft.mode === "update"

  // The folders a copy may land in. Only a copy asks: a change keeps the
  // query where it is, which is the Move dialog's business.
  const folders = useService(
    (signal) =>
      isChange ? Promise.resolve([] as QueryFolder[]) : queryService.listFolders(signal),
    [isChange]
  )

  // The query the tab came from: its title and tags are what a copy starts
  // from, and what a change writes.
  const source = useService(
    (signal) =>
      draft.savedId
        ? queryService.getSaved(draft.savedId, signal)
        : Promise.resolve(null as SavedQuery | null),
    [draft.savedId]
  )

  // The page proposes a name taken from the query itself, so the common case
  // is "type nothing, press Save". A tab that came from a saved query keeps
  // that query's own name and tags instead.
  React.useEffect(() => {
    const found = source.status === "success" ? source.data : null
    setTitle(found ? found.title : sqlSummary(draft.sql))
    setTags(found ? found.tags.join(", ") : "")
  }, [draft.sql, source.status, source.data])

  const tree = React.useMemo(
    () => buildSavedQueryTree(folders.data ?? []),
    [folders.data]
  )
  const personalFolders = React.useMemo(
    () => (tree.personal ? flattenTreeFolders(tree.personal, PERSONAL_SPACE_LABEL) : []),
    [tree]
  )
  const sharedFolders = React.useMemo(() => flattenTreeFolders(tree.shared, ""), [tree])

  /**
   * The folder the query lands in: the caller's own space sends no folder at
   * all, the shared space sends `null`, and a folder sends its id.
   */
  const folderId = React.useMemo(() => {
    if (isChange) return undefined
    if (folder === SHARED_SPACE) return null
    if (folder === PERSONAL_SPACE) return undefined
    return folder
  }, [isChange, folder])

  const noteFolderId =
    typeof folderId === "string" &&
    levelAtLeast(folders.data?.find((candidate) => candidate.id === folderId)?.level, "edit")
      ? folderId
      : null

  const save = useServiceAction((signal, input: SaveQueryInput) => {
    // The rule for "change the saved query or copy it" is the editor's own
    // (`saved-query-write.ts`), so both save the same way.
    const request = savedQueryRequest({ mode: draft.mode, savedId: draft.savedId }, input)
    if (request.kind === "update") {
      return queryService.updateSaved(request.id, request.body, signal)
    }
    return queryService.saveQuery(request.input, signal)
  })

  async function handleSave() {
    const input: SaveQueryInput = {
      title: title.trim(),
      sql: draft.sql,
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
    }
    if (folderId !== undefined) input.folderId = folderId
    // A new query without placeholders sends no parameters at all. A change
    // always states the definitions it wants: a statement without a
    // placeholder wants none, so it sends an empty list and the server
    // clears the definitions with the SQL. What the saved query held does
    // not decide it, and the change does not wait for it to be read
    // (`#422 S1`).
    if (draft.parameters.length > 0) input.parameters = [...draft.parameters]
    else if (isChange) input.parameters = []
    const saved = await save.run(input)
    if (!saved) return
    // The editor is unmounted while this page saves, so it is told afterwards
    // which query the save produced: the tab keeps its "Save" button.
    rememberSavedTab(draft.tabId, saved.id, saved.level)
    clearSaveQueryDraft()
    router.push(backHref)
  }

  return (
    <FormPage
      title="Save query"
      description={
        isChange
          ? "The change is written to the saved query, in the folder it already sits in."
          : "The query is kept in the folder you pick, with you recorded as the author."
      }
      backHref={backHref}
      backLabel="Back to Query Studio"
      submitLabel={isChange ? "Save" : "Save as new"}
      submittingLabel="Saving…"
      canSubmit={title.trim().length > 0 && draft.sql.trim().length > 0}
      submitting={save.status === "pending"}
      error={save.status === "error" ? save.error.message : null}
      onSubmit={() => void handleSave()}
      onCancel={() => router.push(backHref)}
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
      {isChange ? null : (
        <div className="space-y-2">
          <Label htmlFor="saved-query-folder">Folder</Label>
          {/* WEB-9 D3: the two `<optgroup>`s are SelectOptionGroups, so the
              picker still reads as "Personal space" then "Shared". */}
          <SelectField
            id="saved-query-folder"
            className="w-full"
            value={folder}
            options={[
              {
                label: PERSONAL_SPACE_LABEL,
                options: [
                  { value: PERSONAL_SPACE, label: PERSONAL_SPACE_LABEL },
                  ...personalFolders.map((option) => ({ value: option.value, label: option.label })),
                ],
              },
              {
                label: SHARED_SPACE_LABEL,
                options: [
                  { value: SHARED_SPACE, label: SHARED_SPACE_LABEL },
                  ...sharedFolders.map((option) => ({ value: option.value, label: option.label })),
                ],
              },
            ]}
            onValueChange={setFolder}
          />
          {/* Only a real folder can hold rules: the shared space and the
              caller's own space send `null` and `undefined`. The server
              refuses the access list to a caller below edit on the folder,
              so such a folder is not asked (QS-3 review). The key drops the
              previous folder's names at once; the hook keeps them until its
              effect resets (QS-3 review). */}
          <FolderSharingNote
            key={noteFolderId ?? ""}
            folderId={noteFolderId}
            viewerId={user?.id}
          />
        </div>
      )}
    </FormPage>
  )
}
