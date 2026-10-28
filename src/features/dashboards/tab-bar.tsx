"use client";

import * as React from "react";
import { MoreHorizontal, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MAX_TABS, TAB_TITLE_MAX } from "@/lib/dashboard-tabs";
import { cn } from "@/lib/utils";
import { LineDialog } from "./line-dialog";
import type { BoardTab } from "@/services";

/** What an editor can do to the tabs; absent for a viewer, an embed and a public link. */
export type TabEditing = {
  onAdd: (title: string) => void;
  onRename: (id: string, title: string) => void;
  onMove: (id: string, by: -1 | 1) => void;
  /** `moveTiles`: the tab's tiles go to the first remaining tab; otherwise the tab was empty. */
  onDelete: (id: string, moveTiles: boolean) => void;
  tileCount: (id: string) => number;
};

function DeleteTabDialog({
  tab, count, target, onClose, onDelete,
}: { tab: BoardTab; count: number; target?: BoardTab; onClose: () => void; onDelete: (moveTiles: boolean) => void }) {
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>Delete tab “{tab.title}”?</DialogTitle>
          <DialogDescription>
            {count === 0
              ? "The tab is empty."
              : target
                ? `It has ${count} tile${count === 1 ? "" : "s"}. They move to “${target.title}”.`
                : `It has ${count} tile${count === 1 ? "" : "s"}, which stay on the dashboard without tabs.`}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" variant="destructive" onClick={() => onDelete(count > 0)}>
            {count > 0 && target ? "Move tiles and delete" : "Delete tab"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * The tab bar of a dashboard (BI-18 part C). Read-only unless `editing` is
 * given, which adds a ⋯ menu to each tab (rename, move, delete) and a button
 * to add one. A dashboard with no tabs shows nothing to a reader.
 */
export function TabBar({
  tabs, selected, onSelect, editing,
}: { tabs: BoardTab[]; selected?: string; onSelect: (id: string) => void; editing?: TabEditing }) {
  const [dialog, setDialog] = React.useState<{ kind: "add" } | { kind: "rename" | "delete"; tab: BoardTab } | null>(null);
  if (tabs.length === 0 && !editing) return null;
  const close = () => setDialog(null);
  return (
    <div data-print-hide className="flex items-center gap-1 overflow-x-auto border-b border-border" role="tablist" aria-label="Dashboard tabs">
      {tabs.map((t, i) => {
        const active = t.id === selected;
        return (
          <div key={t.id} className={cn("flex shrink-0 items-center border-b-2 -mb-px", active ? "border-primary" : "border-transparent")}>
            <button
              type="button"
              role="tab"
              aria-selected={active}
              className={cn("px-3 py-2 text-sm font-medium", active ? "text-foreground" : "text-muted-foreground hover:text-foreground")}
              onClick={() => onSelect(t.id)}
            >
              {t.title}
            </button>
            {editing ? (
              <DropdownMenu>
                <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" className="size-6" aria-label={`Actions for tab ${t.title}`} />}>
                  <MoreHorizontal className="size-3.5" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="w-44">
                  <DropdownMenuItem onClick={() => setDialog({ kind: "rename", tab: t })}>Rename</DropdownMenuItem>
                  <DropdownMenuItem disabled={i === 0} onClick={() => editing.onMove(t.id, -1)}>Move left</DropdownMenuItem>
                  <DropdownMenuItem disabled={i === tabs.length - 1} onClick={() => editing.onMove(t.id, 1)}>Move right</DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem variant="destructive" onClick={() => setDialog({ kind: "delete", tab: t })}>Delete tab</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
          </div>
        );
      })}
      {editing ? (
        <Button variant="ghost" size="sm" className="shrink-0" disabled={tabs.length >= MAX_TABS} onClick={() => setDialog({ kind: "add" })}>
          <Plus className="size-3.5" /> {tabs.length === 0 ? "Add tabs" : "Tab"}
        </Button>
      ) : null}

      {editing && dialog?.kind === "add" ? (
        <LineDialog title="New tab" initial="" confirm="Add" label="Tab name" maxLength={TAB_TITLE_MAX} onClose={close} onSave={(title) => { editing.onAdd(title); close(); }} />
      ) : null}
      {editing && dialog?.kind === "rename" ? (
        <LineDialog title="Rename tab" initial={dialog.tab.title} confirm="Save" label="Tab name" maxLength={TAB_TITLE_MAX} onClose={close}
                     onSave={(title) => { editing.onRename(dialog.tab.id, title); close(); }} />
      ) : null}
      {editing && dialog?.kind === "delete" ? (
        <DeleteTabDialog
          tab={dialog.tab}
          count={editing.tileCount(dialog.tab.id)}
          target={tabs.find((t) => t.id !== (dialog as { tab: BoardTab }).tab.id)}
          onClose={close}
          onDelete={(moveTiles) => { editing.onDelete(dialog.tab.id, moveTiles); close(); }}
        />
      ) : null}
    </div>
  );
}
