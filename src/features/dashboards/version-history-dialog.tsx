"use client";

import * as React from "react";
import { History } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { useService, useServiceAction } from "@/hooks/use-service";
import { revertNote } from "@/lib/dashboard-organise";
import { formatRelativeTime } from "@/lib/format";
import { notifySuccess } from "@/lib/notify";
import { dashboardService } from "@/services";

/**
 * The last versions of a dashboard (BI-18 part C, part 2): who changed it,
 * when, and what changed. An editor goes back to one; going back is itself a
 * new version, so it can be undone. Mount it only while it is open: it reads
 * the history when it appears.
 *
 * A version that no longer holds against today's rules (a column or metric
 * gone since) is refused by the server with a sentence naming it, shown under
 * the row, and nothing is changed.
 */
export function VersionHistoryDialog({
  boardId, canWrite, onOpenChange, onReverted,
}: {
  readonly boardId: string;
  readonly canWrite: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** After a successful revert, so the page reloads the dashboard. */
  readonly onReverted: () => void;
}) {
  const history = useService((signal) => dashboardService.listVersions(boardId, signal), [boardId]);
  const [asking, setAsking] = React.useState<number | null>(null);
  const revert = useServiceAction((signal, version: number) =>
    dashboardService.revertToVersion(boardId, version, signal)
  );
  const versions = history.data?.versions ?? [];

  async function goBack(version: number) {
    const done = await revert.run(version);
    if (!done) return;
    notifySuccess(`Went back to version ${version}`, revertNote(done) || undefined);
    onOpenChange(false);
    onReverted();
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><History className="size-4" aria-hidden /> Version history</DialogTitle>
        </DialogHeader>
        {history.status === "loading" ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
        {history.status === "error" ? <p className="text-sm text-destructive">{history.error.message}</p> : null}
        {history.status === "success" && versions.length === 0 ? (
          <p className="text-sm text-muted-foreground">No versions yet. The next change to this dashboard is the first.</p>
        ) : null}
        <ul className="flex max-h-[50vh] flex-col divide-y overflow-y-auto">
          {versions.map((v, i) => (
            <li key={v.version} className="flex flex-col gap-1 py-2">
              <div className="flex items-center gap-2 text-sm">
                <span className="font-medium tabular-nums">v{v.version}</span>
                <span className="min-w-0 flex-1 truncate">{v.summary}</span>
                {i === 0 ? (
                  <span className="text-xs text-muted-foreground">Current</span>
                ) : canWrite && asking !== v.version ? (
                  <Button size="xs" variant="outline" onClick={() => { revert.reset(); setAsking(v.version); }}>
                    Go back
                  </Button>
                ) : null}
              </div>
              <p className="text-xs text-muted-foreground">
                {v.author || "Unknown"} · {formatRelativeTime(v.at)} · {v.charts} {v.charts === 1 ? "chart" : "charts"}
              </p>
              {asking === v.version ? (
                <div className="flex flex-wrap items-center gap-2 text-sm">
                  <span>Restore this version?</span>
                  <Button size="xs" disabled={revert.status === "pending"} onClick={() => void goBack(v.version)}>
                    {revert.status === "pending" ? "Restoring…" : "Restore"}
                  </Button>
                  <Button size="xs" variant="ghost" onClick={() => { revert.reset(); setAsking(null); }}>Cancel</Button>
                </div>
              ) : null}
              {asking === v.version && revert.error ? (
                <p role="alert" className="text-sm text-destructive">{revert.error.message}</p>
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
