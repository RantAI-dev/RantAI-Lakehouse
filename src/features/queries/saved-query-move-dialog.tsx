"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { levelAtLeast } from "@/lib/access-level"
import { readableErrorMessage } from "@/lib/notify-message"
import type { QueryFolder } from "@/services/contracts/queries"
import { FolderSharingNote } from "./folder-sharing-note"
import {
  PERSONAL_SPACE_LABEL,
  SHARED_SPACE_LABEL,
  buildSavedQueryTree,
  flattenTreeFolders,
} from "./saved-query-tree"

/** One place a query or a folder may go. */
type MoveTarget = {
  /** What the picker sends: the folder's id, or nothing for the shared space. */
  readonly value: string
  /** What the picker shows: the folder's path from its space. */
  readonly label: string
}

/**
 * The places a thing may go, in the tree's grouping: the caller's own
 * space first, then the shared space and the folders under it.
 *
 * Every folder the server sent is offered, even one the caller may not
 * fill: the server refuses such a move with its own sentence, and the
 * console does not guess the rule before the server answers (C6).
 */
function moveTargets(
  folders: readonly QueryFolder[],
  excludeId?: string,
): { personal: MoveTarget[]; shared: MoveTarget[] } {
  const tree = buildSavedQueryTree(folders)
  const personal: MoveTarget[] = []
  const shared: MoveTarget[] = []
  if (tree.personal) {
    personal.push({ value: tree.personal.id, label: PERSONAL_SPACE_LABEL })
    personal.push(...flattenTreeFolders(tree.personal, PERSONAL_SPACE_LABEL))
  }
  shared.push({ value: "", label: SHARED_SPACE_LABEL })
  shared.push(...flattenTreeFolders(tree.shared, ""))
  const without = (list: MoveTarget[]) =>
    list.filter((target) => target.value !== excludeId)
  return { personal: without(personal), shared: without(shared) }
}

/**
 * Move a saved query, or a folder, into another place (`QS-3` C2).
 *
 * The picker offers the shared space, the caller's own space and the
 * folders, and hands the choice over as the server reads it: an id for a
 * folder, `null` for the shared space. A refusal is the server's sentence,
 * shown as it was given — the console writes none of its own (C6).
 */
export function SavedQueryMoveDialog({
  folders,
  subject,
  kind = "query",
  currentFolderId,
  excludeId,
  onClose,
  onMove,
  busy,
  error,
  viewerId,
}: {
  /** The folders the caller may see, as the server sent them. */
  readonly folders: QueryFolder[]
  /** What is being moved, as the dialog names it. */
  readonly subject: string
  /** What is being moved, for the dialog's title. */
  readonly kind?: "query" | "folder"
  /** The folder it sits in now; `null` for the shared space. */
  readonly currentFolderId: string | null
  /** A folder that may not be a target: the folder that is being moved. */
  readonly excludeId?: string
  readonly onClose: () => void
  /** The target: an id for a folder, `null` for the shared space. */
  readonly onMove: (target: string | null) => void
  readonly busy?: boolean
  /** The refusal, in the server's own words. */
  readonly error?: string | null
  /** The signed-in person, who is left out of "Shared with"; `undefined` leaves nobody out. */
  readonly viewerId: string | undefined
}) {
  const targets = React.useMemo(
    () => moveTargets(folders, excludeId),
    [folders, excludeId],
  )
  const [target, setTarget] = React.useState(currentFolderId ?? "")
  const pickerId = React.useId()
  // The shared space (value "") and the caller's own space hold no rule, so
  // the dialog neither asks nor says anything for them. A folder inside the
  // own space is a real folder and can be shared.
  const personalRootId = folders.find(
    (folder) => folder.isPersonal && folder.parentId === null,
  )?.id
  // The server refuses the access list to a caller below edit on the folder,
  // and the picker offers every folder, so such a folder is not asked
  // (QS-3 review).
  const targetLevel = folders.find((folder) => folder.id === target)?.level
  const sharedFolderId =
    target === "" || target === personalRootId || !levelAtLeast(targetLevel, "edit")
      ? null
      : target

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>
            {kind === "query" ? "Move query" : "Move folder"}
          </DialogTitle>
          <DialogDescription>
            &quot;{subject}&quot; moves into the folder you pick.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-1.5">
          <label htmlFor={pickerId} className="text-sm font-medium">
            Target folder
          </label>
          <select
            id={pickerId}
            value={target}
            onChange={(event) => setTarget(event.target.value)}
            className="h-8 w-full min-w-0 rounded-lg border border-input bg-transparent px-2.5 py-1 text-sm transition-colors outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {targets.personal.length > 0 ? (
              <optgroup label={PERSONAL_SPACE_LABEL}>
                {targets.personal.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </optgroup>
            ) : null}
            <optgroup label={SHARED_SPACE_LABEL}>
              {targets.shared.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </optgroup>
          </select>
          {/* The key drops the previous folder's names at once; the hook
              keeps them until its effect resets (QS-3 review). */}
          <FolderSharingNote
            key={sharedFolderId ?? ""}
            folderId={sharedFolderId}
            viewerId={viewerId}
          />
        </div>
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {readableErrorMessage(error)}
          </p>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={busy}
            onClick={() => onMove(target === "" ? null : target)}
          >
            Move
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
