"use client"

import { FolderIcon, FilesIcon, FolderOpenIcon, InboxIcon } from "lucide-react"
import { buildFolderTree, type FolderNode } from "@/lib/folder-tree"
import { ALL_FILES, NO_FOLDER, asFolderLikes } from "@/lib/upload-folders"
import { cn } from "@/lib/utils"
import type { UploadFolder } from "@/services/contracts/uploads"

type Node = FolderNode<{ id: string; name: string }>

function Entry({
  label,
  active,
  depth,
  count,
  icon,
  onSelect,
}: {
  readonly label: string
  readonly active: boolean
  readonly depth: number
  readonly count?: number
  readonly icon: React.ReactNode
  readonly onSelect: () => void
}) {
  return (
    <li>
      <button
        type="button"
        aria-current={active ? "true" : undefined}
        onClick={onSelect}
        style={{ paddingLeft: `${8 + depth * 14}px` }}
        className={cn(
          "flex w-full min-w-0 items-center gap-2 rounded-md py-1.5 pr-2 text-left text-sm hover:bg-muted",
          active && "bg-muted font-medium"
        )}
      >
        {icon}
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {count !== undefined ? (
          <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{count}</span>
        ) : null}
      </button>
    </li>
  )
}

/**
 * The folder column of the Uploaded files tab (`DATA-35`, plan AD6): "All files", "No folder", then
 * the tenant's folders nested as the API sent them. The number beside a folder is the count of files
 * DIRECTLY in it, from the server (a file in a sub-folder is counted there).
 */
export function UploadFolderTree({
  folders,
  selected,
  onSelect,
}: {
  readonly folders: readonly UploadFolder[]
  /** `ALL_FILES`, `NO_FOLDER` or a folder id. */
  readonly selected: string
  readonly onSelect: (value: string) => void
}) {
  const byId = new Map(folders.map((f) => [f.id, f]))
  const nodes = buildFolderTree(asFolderLikes(folders), []).folders
  const render = (node: Node): React.ReactNode => {
    const folder = byId.get(node.folder.id)
    const active = selected === node.folder.id
    return (
      <li key={node.folder.id}>
        <ul className="space-y-0.5">
          <Entry
            label={node.folder.name}
            active={active}
            depth={node.depth - 1}
            count={folder?.fileCount}
            icon={
              active ? (
                <FolderOpenIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
              ) : (
                <FolderIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
              )
            }
            onSelect={() => onSelect(node.folder.id)}
          />
          {node.children.map(render)}
        </ul>
      </li>
    )
  }
  return (
    <nav aria-label="Folders">
      <ul className="space-y-0.5">
        <Entry
          label="All files"
          active={selected === ALL_FILES}
          depth={0}
          icon={<FilesIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />}
          onSelect={() => onSelect(ALL_FILES)}
        />
        <Entry
          label="No folder"
          active={selected === NO_FOLDER}
          depth={0}
          icon={<InboxIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />}
          onSelect={() => onSelect(NO_FOLDER)}
        />
        {nodes.map(render)}
      </ul>
    </nav>
  )
}
