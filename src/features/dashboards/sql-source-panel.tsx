"use client";

import { Play, TriangleAlert, X } from "lucide-react";
import { SqlEditor } from "@/components/sql-editor";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatCell, sourceNameProblem, type SqlRunPhase } from "@/lib/sql-source-draft";
import type { SqlSourcePreview } from "@/services/contracts/dashboards";
import { cn } from "@/lib/utils";
import type { SqlSourceDraft } from "./use-sql-source-draft";

/** The columns a Run returned, with their types. */
function SqlColumnChips({ result }: { result: SqlSourcePreview }) {
  return (
    <div className="grid gap-1.5">
      <div className="flex flex-wrap gap-1.5" aria-label="Columns">
        {result.columns.map((c) => (
          <span key={c.name} className="rounded border border-border px-1.5 py-0.5 font-mono text-[11px]">
            {c.name} <span className="text-muted-foreground">{c.type}</span>
          </span>
        ))}
      </div>
      {result.rows.length === 0 ? <p className="text-xs text-muted-foreground">The SQL ran and returned no rows.</p> : null}
    </div>
  );
}

/**
 * The first rows a Run returned. Lives in the Preview pane only; the panel
 * under the editor lists just the columns, so the rows are never shown twice.
 */
export function SqlRowsTable({ result, stale, className }: {
  result: SqlSourcePreview;
  /** The SQL was edited after this result: say so, since the rows are of the earlier text. */
  stale?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("grid min-h-0 content-start gap-2", className)}>
      <p className="text-xs text-muted-foreground">
        {result.rows.length === 0
          ? "The SQL ran and returned no rows."
          : `First ${result.rows.length.toLocaleString("en-US")} row${result.rows.length === 1 ? "" : "s"}.`}
        {stale ? " These are the rows of the SQL before your last edit." : ""}
      </p>
      {result.rows.length ? (
        <div className={cn("min-h-0 overflow-auto rounded-md border border-border", stale && "opacity-60")}>
          <table className="w-full text-left text-xs">
            <thead className="sticky top-0 bg-muted">
              <tr>
                {result.columns.map((c) => (
                  <th key={c.name} scope="col" className="px-2 py-1 font-medium whitespace-nowrap">{c.name}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {result.rows.map((row, i) => (
                <tr key={i} className="border-t border-border">
                  {result.columns.map((c) => {
                    const text = formatCell(row[c.name]);
                    return (
                      <td
                        key={c.name} title={text}
                        className={cn("max-w-56 truncate px-2 py-1 font-mono", row[c.name] == null && "text-muted-foreground italic")}
                      >
                        {text}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </div>
  );
}

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
          <span className="font-mono">serving.&lt;table&gt;</span>. At most 2,000 rows and 30 seconds.
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
