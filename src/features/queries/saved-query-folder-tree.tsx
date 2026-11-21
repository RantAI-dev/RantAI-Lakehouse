"use client"

import * as React from "react"
import {
  FolderIcon,
  FolderInputIcon,
  FolderPlusIcon,
  LockIcon,
  PencilIcon,
  Share2Icon,
  Trash2Icon,
} from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { levelAtLeast } from "@/lib/access-level"
import { useServiceAction } from "@/hooks/use-service"
import { MAX_FOLDER_DEPTH } from "@/lib/folder-tree"
import { readableErrorMessage } from "@/lib/notify-message"
import { queryService } from "@/services"
import type { QueryFolder } from "@/services/contracts/queries"
import {
  SavedQueryFolderNameDialog,
  type SavedQueryFolderNaming,
} from "./saved-query-folder-dialogs"
import { SavedQueryAccessPanel } from "./saved-query-access-panel"
import { SavedQueryMoveDialog } from "./saved-query-move-dialog"
import { buildSavedQueryTree, type SavedQueryTreeNode } from "./saved-query-tree"

/**
 * The tree of the saved-query spaces (`QS-3` C1): the caller's own space,
 * the shared space and the folders under each. A node lists the queries it
 * holds, and the folder actions sit beside it.
 *
 * The level the server sends with each folder decides what the tree offers
 * (C2): a folder may be filled at **edit**, renamed at **edit**, moved,
 * shared and deleted at **manage**. A personal space is never renamed, moved,
 * deleted or shared — it is one person's own space — so the tree offers it
 * none of those.
 */
export function SavedQueryFolderTree({
  folders,
  selected,
  onSelect,
  onChanged,
  viewerId,
}: {
  /** The folders the caller may see, as the server sent them. */
  readonly folders: QueryFolder[]
  /** The `folder` value the list is asked for; `undefined` for every query. */
  readonly selected: string | undefined
  readonly onSelect: (folder: string | undefined) => void
  /** Called after a change, so the folders and the list are read again. */
  readonly onChanged: () => void
  /** The signed-in person, for the move dialog's "Shared with" line. */
  readonly viewerId: string | undefined
}) {
  const tree = React.useMemo(() => buildSavedQueryTree(folders), [folders])
  const createAct = useServiceAction((signal, name: string, parentId: string | null) =>
    queryService.createFolder({ name, parentId }, signal),
  )
  const renameAct = useServiceAction((signal, id: string, name: string) =>
    queryService.updateFolder(id, { name }, signal),
  )
  // A move writes the folder the folder lands in: `null` is the shared
  // space, an id is that folder. The server refuses a move below manage,
  // and says so in its own sentence.
  const moveAct = useServiceAction((signal, id: string, parentId: string | null) =>
    queryService.updateFolder(id, { parentId }, signal),
  )
  const deleteAct = useServiceAction((signal, id: string) =>
    queryService.deleteFolder(id, signal).then(() => true),
  )
  // Each refusal is said inside the dialog the person is looking at: the
  // rename and the create in their own, the delete in its confirm dialog.
  // A sentence said twice is a sentence nobody reads (C6).
  const { run: runMove, reset: resetMove } = moveAct
  const { reset: resetDelete } = deleteAct
  const [naming, setNaming] = React.useState<SavedQueryFolderNaming | null>(null)
  const [moving, setMoving] = React.useState<QueryFolder | null>(null)
  const [deleting, setDeleting] = React.useState<QueryFolder | null>(null)
  const [sharing, setSharing] = React.useState<QueryFolder | null>(null)

  async function create(name: string) {
    if (!naming || naming.mode !== "create") return
    const done = await createAct.run(name, naming.parentId)
    if (done) {
      setNaming(null)
      onChanged()
    }
  }

  async function rename(name: string) {
    if (!naming || naming.mode !== "rename") return
    const done = await renameAct.run(naming.id, name)
    if (done) {
      setNaming(null)
      onChanged()
    }
  }

  async function remove() {
    if (!deleting) return
    const done = await deleteAct.run(deleting.id)
    if (done) {
      setDeleting(null)
      onChanged()
    }
  }

  async function move(parentId: string | null) {
    if (!moving) return
    const done = await runMove(moving.id, parentId)
    if (done) {
      setMoving(null)
      onChanged()
    }
  }

  const startRename = (folder: QueryFolder) =>
    setNaming({ mode: "rename", id: folder.id, current: folder.name })

  const startMove = (folder: QueryFolder) => {
    resetMove()
    setMoving(folder)
  }

  // The refusal of the last delete belongs to that attempt, so a dialog
  // that opens again starts without it (#389 SHOULD-FIX 1).
  const startDelete = (folder: QueryFolder) => {
    resetDelete()
    setDeleting(folder)
  }

  const startCreate = (parentId: string | null, place: string) =>
    setNaming({ mode: "create", parentId, place })

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-medium">Folders</h2>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => startCreate(null, "the shared space")}
        >
          <FolderPlusIcon className="size-4" aria-hidden />
          New folder
        </Button>
      </div>

      <nav aria-label="Saved query folders">
        <ul className="space-y-0.5">
          <li>
            <button
              type="button"
              onClick={() => onSelect(undefined)}
              aria-current={selected === undefined ? "true" : undefined}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-muted"
            >
              All queries
            </button>
          </li>
          {tree.personal ? (
            <FolderTreeNode
              node={tree.personal}
              selected={selected}
              onSelect={onSelect}
              onCreate={startCreate}
              onRename={startRename}
              onMove={startMove}
              onDelete={startDelete}
              onShare={setSharing}
            />
          ) : null}
          <FolderTreeNode
            node={tree.shared}
            selected={selected}
            onSelect={onSelect}
            onCreate={startCreate}
            onRename={startRename}
            onMove={startMove}
            onDelete={startDelete}
            onShare={setSharing}
          />
        </ul>
      </nav>

      {naming ? (
        <SavedQueryFolderNameDialog
          key={naming.mode === "rename" ? naming.id : `new-${naming.parentId ?? "root"}`}
          naming={naming}
          onClose={() => setNaming(null)}
          onSubmit={(name) => {
            if (naming.mode === "rename") void rename(name)
            else void create(name)
          }}
          busy={createAct.status === "pending" || renameAct.status === "pending"}
          error={
            createAct.error || renameAct.error
              ? readableErrorMessage(
                  (createAct.error ?? renameAct.error)?.message ?? "",
                )
              : null
          }
        />
      ) : null}

      {deleting ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => {
            if (!open) setDeleting(null)
          }}
          title="Delete folder"
          description={`"${deleting.name}" leaves the tree. It must hold no queries and no folders.`}
          confirmLabel="Delete"
          destructive
          confirming={deleteAct.status === "pending"}
          onConfirm={() => void remove()}
        >
          {deleteAct.error ? (
            <p role="alert" className="text-sm text-destructive">
              {readableErrorMessage(deleteAct.error.message)}
            </p>
          ) : null}
        </ConfirmActionDialog>
      ) : null}

      {moving ? (
        <SavedQueryMoveDialog
          key={moving.id}
          folders={folders}
          subject={moving.name}
          kind="folder"
          currentFolderId={moving.parentId}
          excludeId={moving.id}
          onClose={() => setMoving(null)}
          onMove={(target) => void move(target)}
          busy={moveAct.status === "pending"}
          error={moveAct.error?.message ?? null}
          viewerId={viewerId}
        />
      ) : null}

      {sharing ? (
        <SavedQueryAccessPanel
          key={sharing.id}
          kind="folder"
          id={sharing.id}
          title={sharing.name}
          onClose={() => setSharing(null)}
        />
      ) : null}
    </div>
  )
}

/**
 * One node of the tree and the folders under it.
 *
 * The shared space has no node of its own on the server, so it carries no
 * level and offers no folder action of its own: a folder in it is made
 * with "New folder" above the tree.
 */
function FolderTreeNode({
  node,
  selected,
  onSelect,
  onCreate,
  onRename,
  onMove,
  onDelete,
  onShare,
}: {
  readonly node: SavedQueryTreeNode
  readonly selected: string | undefined
  readonly onSelect: (folder: string | undefined) => void
  readonly onCreate: (parentId: string | null, place: string) => void
  readonly onRename: (folder: QueryFolder) => void
  readonly onMove: (folder: QueryFolder) => void
  readonly onDelete: (folder: QueryFolder) => void
  readonly onShare: (folder: QueryFolder) => void
}) {
  const folder = node.record
  const isShared = node.id === "shared"
  // Filling and renaming a folder need **edit**; moving, sharing and
  // deleting one need **manage**. A personal space is never renamed,
  // moved, deleted or shared (C2): it is one person's own space, and a
  // rule on it would reach everything inside it.
  const mayFill = !isShared && levelAtLeast(node.level, "edit")
  const mayChange = mayFill && !node.isPersonal
  const mayManage = !isShared && levelAtLeast(node.level, "manage") && !node.isPersonal
  const mayNest = node.depth < MAX_FOLDER_DEPTH

  return (
    <li>
      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={() => onSelect(node.folder)}
          aria-current={selected === node.folder ? "true" : undefined}
          title={node.name}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-muted"
        >
          {node.isPersonal ? (
            <LockIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          ) : (
            <FolderIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          )}
          <span className="truncate">{node.label}</span>
        </button>
        {mayFill || mayChange || mayManage ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7"
                  aria-label={`Actions for ${node.label}`}
                />
              }
            >
              <FolderPlusIcon className="size-4" aria-hidden />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {mayFill && mayNest ? (
                <DropdownMenuItem
                  onClick={() =>
                    onCreate(
                      node.id,
                      node.isPersonal ? "your personal space" : `"${node.label}"`,
                    )
                  }
                >
                  <FolderPlusIcon className="size-3.5" data-icon="inline-start" />
                  {node.isPersonal ? "New folder" : "New subfolder"}
                </DropdownMenuItem>
              ) : null}
              {mayChange && folder ? (
                <DropdownMenuItem onClick={() => onRename(folder)}>
                  <PencilIcon className="size-3.5" data-icon="inline-start" />
                  Rename
                </DropdownMenuItem>
              ) : null}
              {mayManage && folder ? (
                <>
                  <DropdownMenuItem onClick={() => onMove(folder)}>
                    <FolderInputIcon className="size-3.5" data-icon="inline-start" />
                    Move
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => onShare(folder)}>
                    <Share2Icon className="size-3.5" data-icon="inline-start" />
                    Share
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    variant="destructive"
                    onClick={() => onDelete(folder)}
                  >
                    <Trash2Icon className="size-3.5" data-icon="inline-start" />
                    Delete
                  </DropdownMenuItem>
                </>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
      {node.children.length > 0 ? (
        <ul className="ml-3 space-y-0.5 border-l pl-2">
          {node.children.map((child) => (
            <FolderTreeNode
              key={child.id}
              node={child}
              selected={selected}
              onSelect={onSelect}
              onCreate={onCreate}
              onRename={onRename}
              onMove={onMove}
              onDelete={onDelete}
              onShare={onShare}
            />
          ))}
        </ul>
      ) : null}
    </li>
  )
}
