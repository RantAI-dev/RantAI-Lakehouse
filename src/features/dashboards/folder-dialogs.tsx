"use client";

import * as React from "react";
import { Check, Folder, FolderPlus, Pencil, Trash2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { useServiceAction } from "@/hooks/use-service";
import {
  MAX_FOLDER_DEPTH, buildFolderTree, flattenFolders, folderPath, type FolderLike,
} from "@/lib/folder-tree";
import { dashboardService } from "@/services";

/** Select value for "no folder" (base-ui needs a non-empty value). */
const ROOT = "__root__";

/**
 * Create, rename, nest and delete dashboard folders. The server enforces
 * the rules (four levels, no cycles, unique sibling names, delete only when
 * empty) and answers with fixed messages, which are shown as-is.
 */
export function ManageFoldersDialog({
  open, onOpenChange, folders, onChanged,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly folders: FolderLike[];
  /** Called after every successful change so the caller reloads. */
  readonly onChanged: () => void;
}) {
  const [newName, setNewName] = React.useState("");
  const [newParent, setNewParent] = React.useState(ROOT);
  const [editing, setEditing] = React.useState<{ id: string; name: string } | null>(null);
  const flat = flattenFolders(buildFolderTree(folders, []).folders);

  const createAct = useServiceAction((signal, name: string, parentId: string) =>
    dashboardService.createFolder({ name, parentId }, signal)
  );
  const renameAct = useServiceAction((signal, id: string, name: string) =>
    dashboardService.updateFolder({ id, name }, signal)
  );
  const deleteAct = useServiceAction((signal, id: string) =>
    dashboardService.deleteFolder(id, signal).then(() => true)
  );
  const error = createAct.error ?? renameAct.error ?? deleteAct.error;
  const busy = [createAct, renameAct, deleteAct].some((a) => a.status === "pending");

  React.useEffect(() => {
    if (open) { setNewName(""); setNewParent(ROOT); setEditing(null); }
  }, [open]);

  async function create() {
    const name = newName.trim();
    if (!name) return;
    const done = await createAct.run(name, newParent === ROOT ? "" : newParent);
    if (done) { setNewName(""); onChanged(); }
  }
  async function rename() {
    if (!editing?.name.trim()) return;
    const done = await renameAct.run(editing.id, editing.name.trim());
    if (done) { setEditing(null); onChanged(); }
  }
  async function remove(id: string) {
    if (await deleteAct.run(id)) onChanged();
  }

  const parentOptions = flat.filter((f) => f.depth < MAX_FOLDER_DEPTH);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Folders</DialogTitle>
          <DialogDescription>
            Organize dashboards and SQL sources. Folders nest up to {MAX_FOLDER_DEPTH} levels; only empty folders can be deleted.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-2">
          <div className="flex flex-col gap-2 sm:flex-row">
            <Input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") void create(); }}
              placeholder="New folder name"
              aria-label="New folder name"
            />
            <Select value={newParent} onValueChange={(v) => setNewParent(v ?? ROOT)}>
              <SelectTrigger className="sm:w-48">
                <SelectValue>
                  {newParent === ROOT ? "At the top level" : `In ${folderPath(folders, newParent)}`}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ROOT}>At the top level</SelectItem>
                {parentOptions.map(({ folder }) => (
                  <SelectItem key={folder.id} value={folder.id}>In {folderPath(folders, folder.id)}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button size="sm" onClick={() => void create()} disabled={busy || !newName.trim()}>
              <FolderPlus className="size-4" aria-hidden /> Add
            </Button>
          </div>

          <ul className="max-h-72 divide-y divide-border overflow-y-auto rounded-md border border-border">
            {flat.length === 0 ? (
              <li className="px-3 py-6 text-center text-sm text-muted-foreground">No folders yet.</li>
            ) : flat.map(({ folder, depth }) => (
              <li key={folder.id} className="flex items-center gap-2 px-3 py-1.5" style={{ paddingLeft: `${depth * 16}px` }}>
                <Folder className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                {editing?.id === folder.id ? (
                  <>
                    <Input
                      autoFocus
                      className="h-7"
                      value={editing.name}
                      onChange={(e) => setEditing({ id: folder.id, name: e.target.value })}
                      onKeyDown={(e) => { if (e.key === "Enter") void rename(); if (e.key === "Escape") setEditing(null); }}
                      aria-label="Folder name"
                    />
                    <Button size="icon-sm" variant="ghost" aria-label="Save name" onClick={() => void rename()} disabled={busy}>
                      <Check className="size-4" />
                    </Button>
                    <Button size="icon-sm" variant="ghost" aria-label="Cancel" onClick={() => setEditing(null)}>
                      <X className="size-4" />
                    </Button>
                  </>
                ) : (
                  <>
                    <span className="min-w-0 flex-1 truncate text-sm">{folder.name}</span>
                    <Button size="icon-sm" variant="ghost" aria-label={`Rename ${folder.name}`} onClick={() => setEditing({ id: folder.id, name: folder.name })}>
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button size="icon-sm" variant="ghost" aria-label={`Delete ${folder.name}`} onClick={() => void remove(folder.id)} disabled={busy}>
                      <Trash2 className="size-3.5" />
                    </Button>
                  </>
                )}
              </li>
            ))}
          </ul>
          {error ? <p className="text-sm text-destructive">{error.message}</p> : null}
        </div>

        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Move a dashboard into a folder (or back to the top level). */
export function MoveBoardDialog({
  open, onOpenChange, folders, boardId, currentFolderId, onMoved,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly folders: FolderLike[];
  readonly boardId: string;
  readonly currentFolderId: string;
  readonly onMoved: () => void;
}) {
  const [target, setTarget] = React.useState(currentFolderId || ROOT);
  const moveAct = useServiceAction((signal, folderId: string) =>
    dashboardService.moveBoard(boardId, folderId, signal).then(() => true)
  );
  const flat = flattenFolders(buildFolderTree(folders, []).folders);

  React.useEffect(() => { if (open) setTarget(currentFolderId || ROOT); }, [open, currentFolderId]);

  async function move() {
    const done = await moveAct.run(target === ROOT ? "" : target);
    if (done) { onOpenChange(false); onMoved(); }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>Move to folder</DialogTitle></DialogHeader>
        <Select value={target} onValueChange={(v) => setTarget(v ?? ROOT)}>
          <SelectTrigger>
            <SelectValue>{target === ROOT ? "Top level (no folder)" : folderPath(folders, target)}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ROOT}>Top level (no folder)</SelectItem>
            {flat.map(({ folder }) => (
              <SelectItem key={folder.id} value={folder.id}>{folderPath(folders, folder.id)}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        {flat.length === 0 ? (
          <p className="text-xs text-muted-foreground">No folders yet. Create one with Folders on the dashboard list.</p>
        ) : null}
        {moveAct.error ? <p className="text-sm text-destructive">{moveAct.error.message}</p> : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" onClick={() => void move()} disabled={moveAct.status === "pending" || (target === ROOT ? "" : target) === currentFolderId}>
            Move
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
