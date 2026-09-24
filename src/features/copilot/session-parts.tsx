"use client";

import * as React from "react";
import Link from "next/link";
import { Check, MessageSquare, MoreHorizontal, Pencil, Plus, Search, Sparkles, Trash2 } from "lucide-react";

import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { formatRelativeTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { SessionMeta } from "./use-copilot";

/** Build conversations can change the lakehouse; Ask ones only read it. */
export function SessionModeIcon({ mode, className }: { readonly mode: string; readonly className?: string }) {
  const build = mode === "build";
  return (
    <span
      title={build ? "Build mode" : "Ask mode"}
      className={cn(
        "grid size-7 shrink-0 place-items-center rounded-md",
        build ? "bg-amber-500/10 text-amber-600 dark:text-amber-400" : "bg-primary/10 text-primary",
        className
      )}
    >
      {build ? <Sparkles className="size-3.5" /> : <MessageSquare className="size-3.5" />}
      <span className="sr-only">{build ? "Build" : "Ask"}</span>
    </span>
  );
}

/**
 * The recent-conversations list shared by the header history menus: recent
 * sessions, then "View all history" and "New chat". Managing a session
 * (rename, delete) lives on the history page, not in a menu row.
 */
export function RecentSessionsMenuContent({
  sessions,
  activeId,
  onSelect,
  onNew,
  limit = 8,
}: {
  readonly sessions: SessionMeta[];
  readonly activeId: string | null;
  readonly onSelect: (id: string) => void;
  readonly onNew: () => void;
  readonly limit?: number;
}) {
  return (
    <>
      {sessions.length > 0 ? (
        <DropdownMenuGroup className="max-h-72 overflow-y-auto">
          {sessions.slice(0, limit).map((s) => (
            <DropdownMenuItem key={s.id} onClick={() => onSelect(s.id)} className="items-start gap-2 py-2">
              <Check
                className={cn("mt-0.5 size-3.5 shrink-0", s.id === activeId ? "text-primary" : "opacity-0")}
                aria-hidden
              />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-xs font-medium text-foreground">{s.title}</span>
                <span className="text-[11px] text-muted-foreground">
                  {s.mode === "build" ? "Build" : "Ask"}
                  {s.updatedAt ? ` · ${formatRelativeTime(s.updatedAt)}` : null}
                </span>
              </span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      ) : (
        <div className="px-3 py-4 text-center text-xs text-muted-foreground">No conversations yet</div>
      )}
      <DropdownMenuSeparator />
      <DropdownMenuItem render={<Link href="/copilot/history" />} className="text-xs">
        <Search className="size-3.5" />
        View all history
      </DropdownMenuItem>
      <DropdownMenuItem onClick={onNew} className="text-xs">
        <Plus className="size-3.5" />
        New chat
      </DropdownMenuItem>
    </>
  );
}

/** Rename one conversation; the new title sticks through later messages. */
export function RenameSessionDialog({
  session,
  open,
  onOpenChange,
  onRename,
}: {
  readonly session: SessionMeta;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onRename: (title: string) => Promise<boolean>;
}) {
  const [title, setTitle] = React.useState(session.title);
  const [busy, setBusy] = React.useState(false);
  const clean = title.trim();

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!clean || clean === session.title) return onOpenChange(false);
    setBusy(true);
    const ok = await onRename(clean);
    setBusy(false);
    if (ok) onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={(e) => void submit(e)} className="space-y-4">
          <DialogHeader>
            <DialogTitle>Rename conversation</DialogTitle>
            <DialogDescription>Shown in your history and the Copilot header.</DialogDescription>
          </DialogHeader>
          <Input
            autoFocus
            value={title}
            maxLength={120}
            onChange={(e) => setTitle(e.target.value)}
            aria-label="Conversation title"
          />
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !clean}>
              {busy ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** The ⋯ menu on a conversation row: Rename and Delete (confirmed first). */
export function SessionActionsMenu({
  session,
  onRename,
  onDelete,
}: {
  readonly session: SessionMeta;
  readonly onRename: (title: string) => Promise<boolean>;
  readonly onDelete: () => Promise<boolean>;
}) {
  const [renaming, setRenaming] = React.useState(false);
  const [deleting, setDeleting] = React.useState(false);
  const [busy, setBusy] = React.useState(false);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon"
              className="size-8 text-muted-foreground hover:text-foreground"
              aria-label={`Actions for ${session.title}`}
            />
          }
        >
          <MoreHorizontal className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-40">
          <DropdownMenuItem onClick={() => setRenaming(true)}>
            <Pencil />
            Rename
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onClick={() => setDeleting(true)}>
            <Trash2 />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {renaming ? (
        <RenameSessionDialog session={session} open={renaming} onOpenChange={setRenaming} onRename={onRename} />
      ) : null}
      <ConfirmActionDialog
        open={deleting}
        onOpenChange={setDeleting}
        title="Delete conversation?"
        description={`"${session.title}" and all of its messages will be removed. This can't be undone.`}
        confirmLabel="Delete"
        destructive
        confirming={busy}
        onConfirm={async () => {
          setBusy(true);
          const ok = await onDelete();
          setBusy(false);
          if (ok) setDeleting(false);
        }}
      />
    </>
  );
}
