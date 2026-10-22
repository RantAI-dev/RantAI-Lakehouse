"use client";

import * as React from "react";
import { BarChart3, LayoutGrid, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { useService, useServiceAction } from "@/hooks/use-service";
import { daysLeftLabel, restoreBlocked, restoreNote } from "@/lib/dashboard-organise";
import { notifySuccess } from "@/lib/notify";
import { dashboardService } from "@/services";
import type { TrashItem } from "@/services/contracts/dashboards";

/**
 * The trash (BI-18 part C, part 2): deleted dashboards and charts, kept for 30
 * days. Restoring needs the permission that deletes (`dashboard:write`), so a
 * reader sees the list without the button. Mount it only while it is open.
 */
export function TrashDialog({
  canWrite, onOpenChange, onRestored,
}: {
  readonly canWrite: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** After something came back, so the page reloads its lists. */
  readonly onRestored: () => void;
}) {
  const trash = useService((signal) => dashboardService.listTrash(signal), []);
  const restore = useServiceAction((signal, item: TrashItem) =>
    dashboardService.restoreFromTrash(item.kind, item.id, signal)
  );
  const [failed, setFailed] = React.useState<string | null>(null);
  const [now] = React.useState(() => Date.now());
  const items = trash.data?.items ?? [];

  async function bringBack(item: TrashItem) {
    setFailed(null);
    const done = await restore.run(item);
    if (!done) { setFailed(item.id); return; }
    notifySuccess(`${item.name} restored`);
    trash.reload();
    onRestored();
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><Trash2 className="size-4" aria-hidden /> Trash</DialogTitle>
        </DialogHeader>
        {trash.status === "loading" ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
        {trash.status === "error" ? <p className="text-sm text-destructive">{trash.error.message}</p> : null}
        {trash.status === "success" && items.length === 0 ? (
          <p className="text-sm text-muted-foreground">The trash is empty. Deleted items stay here for {trash.data.retentionDays} days.</p>
        ) : null}
        <ul className="flex max-h-[50vh] flex-col divide-y overflow-y-auto">
          {items.map((item) => (
            <li key={`${item.kind}:${item.id}`} className="flex flex-col gap-1 py-2">
              <div className="flex items-center gap-2 text-sm">
                {item.kind === "board"
                  ? <LayoutGrid className="size-4 shrink-0 text-muted-foreground" aria-label="Dashboard" />
                  : <BarChart3 className="size-4 shrink-0 text-muted-foreground" aria-label="Chart" />}
                <span className="min-w-0 flex-1 truncate font-medium">{item.name}</span>
                {canWrite ? (
                  <Button size="xs" variant="outline" disabled={restore.status === "pending" || restoreBlocked(item)}
                          title={restoreBlocked(item) ? restoreNote(item) : undefined} onClick={() => void bringBack(item)}>
                    Restore
                  </Button>
                ) : null}
              </div>
              <p className="text-xs text-muted-foreground">
                {restoreNote(item)} · {item.deletedBy ? `by ${item.deletedBy} · ` : ""}{daysLeftLabel(item.expiresAt, now)}
              </p>
              {failed === item.id && restore.error ? (
                <p role="alert" className="text-sm text-destructive">{restore.error.message}</p>
              ) : null}
            </li>
          ))}
        </ul>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
