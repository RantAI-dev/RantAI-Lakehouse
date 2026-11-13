"use client";

import { useEffectiveLimits } from "@/hooks/use-effective-limits";
import { Play, TriangleAlert, X } from "lucide-react";
import { SqlEditor } from "@/components/sql-editor";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SqlColumnChips } from "@/components/patterns/sql-rows-table";
import { sourceNameProblem, type SqlRunPhase } from "@/lib/sql-source-draft";
import { cn } from "@/lib/utils";
import type { SqlSourceDraft } from "./use-sql-source-draft";


const PHASE_NOTE: Partial<Record<SqlRunPhase, string>> = {
  unrun: "Run the SQL to see its columns and rows.",
  stale: "The SQL changed since it was run. Run the SQL again before saving; the pickers below still list the earlier columns.",
};

/**
 * Inline SQL panel of the chart builder: write (or edit) the source's
 * SELECT, Run it to see its columns and rows, and name it. Nothing is saved
 * here; the builder saves the source and the chart together.
 *
 * Run is bound to ⌘/Ctrl+Enter inside the panel. The capture phase
 * intercepts it before CodeMirror, whose own binding for that chord inserts
 * a blank line.
 */
export function SqlSourcePanel({ draft, onCancelEdit }: { draft: SqlSourceDraft; onCancelEdit: () => void }) {
  // BI-5: the sentence about the limits follows the setting; without it, no number is printed.
  const limits = useEffectiveLimits()
  const editing = draft.mode === "edit";
  const nameProblem = sourceNameProblem(draft.name);
  // Red only once the user has touched the field or tried to save; before
  // that a missing name is just a quiet hint, not a mistake.
  const nameInvalid = nameProblem !== null && draft.showNameError;
  // An untouched existing source has nothing to run yet, so no prompt to.
  const note = draft.phase === "unrun" && !draft.changed && editing ? undefined : PHASE_NOTE[draft.phase];
  return (
    <div
      className="grid gap-3 rounded-md border border-border bg-muted/20 p-3"
      onKeyDownCapture={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          e.stopPropagation();
          void draft.run();
        }
      }}
    >
      {editing ? (
        <div className="flex items-center justify-between gap-2">
          <p className="min-w-0 truncate text-sm font-medium">Editing SQL source “{draft.original?.title}”</p>
          <Button type="button" size="sm" variant="ghost" onClick={onCancelEdit}>
            <X className="size-3.5" aria-hidden /> Cancel edit
          </Button>
        </div>
      ) : null}
      {editing ? (
        <p role="note" className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-400">
          <TriangleAlert className="mt-px size-3.5 shrink-0" aria-hidden />
          <span>
            {draft.usage.kind === "known" && draft.usage.count > 0
              ? `${draft.usage.count} other chart${draft.usage.count === 1 ? " uses" : "s use"} this source and will change too.`
              : draft.usage.kind === "known"
                ? "No other chart uses this source."
                : "Other charts that use this source will change too."}
          </span>
        </p>
      ) : null}

      <div className="grid gap-1.5">
        <Label>SQL <span aria-hidden className="text-destructive">*</span></Label>
        <SqlEditor value={draft.sql} onChange={draft.setSql} minHeight="200px" wrap />
        <p className="text-xs text-muted-foreground" id="sql-source-hint">
          One read-only SELECT over <span className="font-mono">serving</span> tables, written as{" "}
          <span className="font-mono">serving.&lt;table&gt;</span>. {limits
            ? `At most ${limits.chartRows.toLocaleString("en-US")} rows and ${limits.timeoutSeconds.toLocaleString("en-US")} seconds.`
            : "Row and time limits are set in Admin > Settings."}
          Drill-down to records is not available for SQL sources.
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" size="sm" onClick={() => void draft.run()} disabled={draft.running || draft.phase === "empty"}>
          <Play className="size-4" aria-hidden />
          {draft.running ? "Running…" : "Run"}
        </Button>
        <span className="text-xs text-muted-foreground">⌘/Ctrl+Enter to run</span>
      </div>

      {draft.runError ? <p role="alert" className="text-sm text-destructive">{draft.runError}</p> : null}
      {!draft.runError && note && draft.phase !== "empty" ? (
        <p role="status" className={cn("text-xs", draft.phase === "stale" ? "text-amber-700 dark:text-amber-400" : "text-muted-foreground")}>{note}</p>
      ) : null}
      {draft.result && draft.phase !== "empty" ? (
        <SqlColumnChips result={draft.result} />
      ) : null}

      <div className="grid gap-1.5">
        <Label htmlFor="ch-source-name">Source name <span aria-hidden className="text-destructive">*</span></Label>
        <Input
          id="ch-source-name" value={draft.name} onChange={(e) => draft.setName(e.target.value)}
          placeholder="e.g. Visitors by region and month"
          aria-invalid={nameInvalid ? true : undefined}
        />
        {nameProblem ? (
          <p className={cn("text-xs", nameInvalid ? "text-destructive" : "text-muted-foreground")}>{nameProblem}</p>
        ) : draft.nameFollowsChart ? (
          <p className="text-xs text-muted-foreground">Follows the chart title until you type a name.</p>
        ) : null}
      </div>
    </div>
  );
}
