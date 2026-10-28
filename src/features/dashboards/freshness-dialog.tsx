"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { draftOf, draftProblem, ruleOf, ruleLabel, type CacheRule } from "@/lib/result-cache";
import { FreshnessField } from "./freshness-field";

/**
 * "Freshness…" of the dashboard's ⋯ menu (`BI-4` part B, feature decision 2):
 * how long this dashboard's tiles may be served from remembered results. A
 * chart's own setting (in its builder) wins over this, and this wins over the
 * install's (Admin > Settings). A change in the data under a tile makes it
 * compute again whatever is chosen here.
 */
export function FreshnessDialog({
  open, onOpenChange, rule, effective, saving, error, onSave,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** The dashboard's own rule; absent follows the install's. */
  readonly rule?: CacheRule | null;
  /** The rule in force for the dashboard now (its own, else the install's). */
  readonly effective?: CacheRule | null;
  readonly saving?: boolean;
  /** The server's sentence when the last save was refused. */
  readonly error?: string | null;
  readonly onSave: (rule: CacheRule) => void;
}) {
  const [draft, setDraft] = React.useState(draftOf(rule));
  const problem = draftProblem(draft);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>Freshness</DialogTitle></DialogHeader>
        <div className="grid gap-3">
          <FreshnessField id="board-freshness" draft={draft} onChange={setDraft}
            help="How long this dashboard's results are remembered. A chart's own setting wins over this one." />
          {(rule?.mode ?? "default") === "default" && effective ? <p className="text-xs text-muted-foreground">Now: {ruleLabel(effective)} (the install&apos;s setting).</p> : null}
          <p className="text-xs text-muted-foreground">Results are always computed again when the data under them changes.</p>
          {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" disabled={saving || problem !== null} onClick={() => onSave(ruleOf(draft))}>{saving ? "Saving…" : "Save"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
