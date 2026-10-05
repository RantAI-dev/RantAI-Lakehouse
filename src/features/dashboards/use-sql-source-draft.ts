"use client";

import * as React from "react";
import { useServiceAction } from "@/hooks/use-service";
import {
  otherChartsUsing, saveBlocker, sourceNameFor, sourceNameProblem, sqlChanged, sqlRunPhase,
} from "@/lib/sql-source-draft";
import { hasStatement } from "@/lib/sql-text";
import { dashboardService } from "@/services";
import type { SqlSource, SqlSourceColumn } from "@/services/contracts/dashboards";
import { apiFetch } from "@/services/http";

/** How many OTHER charts an edit of the source would change, as far as the console can tell. */
export type SourceUsage = { kind: "loading" } | { kind: "unknown" } | { kind: "known"; count: number };

/**
 * State of the chart builder's inline SQL panel: a brand-new source or the
 * edit of an existing one. The builder owns the chart; this owns the SQL
 * text, the name, the last Run and what that Run says about the text now
 * (`lib/sql-source-draft`), and nothing is written to the API from here, so
 * a draft that is cancelled leaves no trace.
 */
export function useSqlSourceDraft({ chartTitle, editChartId, onColumns }: {
  /** Title the chart would be saved under: a new source's name follows it until typed. */
  chartTitle: string;
  /** The stored chart being edited, left out of the "other charts" count. */
  editChartId?: string;
  /** Called with the columns of every successful Run, to fill the builder's pickers. */
  onColumns: (columns: SqlSourceColumn[]) => void;
}) {
  const [mode, setMode] = React.useState<"none" | "new" | "edit">("none");
  const [original, setOriginal] = React.useState<SqlSource | null>(null);
  const [sql, setSql] = React.useState("");
  // null = follow the fallback name; any string, even "", was typed by the user.
  const [typedName, setTypedName] = React.useState<string | null>(null);
  const [ranSql, setRanSql] = React.useState<string | null>(null);
  // A save that was tried while the name was missing: only then, or once the
  // user has typed in the field, is the missing name shown as an error.
  const [saveAttempted, setSaveAttempted] = React.useState(false);
  const [usage, setUsage] = React.useState<SourceUsage>({ kind: "loading" });
  const act = useServiceAction((signal, text: string) => dashboardService.previewSqlSource(text, undefined, signal));
  const runSeq = React.useRef(0);
  const onColumnsRef = React.useRef(onColumns);
  React.useEffect(() => { onColumnsRef.current = onColumns; });

  const originalSql = original?.sql ?? "";
  const name = sourceNameFor(typedName, mode === "edit" ? original?.title ?? "" : chartTitle);
  const phase = sqlRunPhase({ act: act.status, ranSql, sql });
  const changed = mode === "edit" && sqlChanged(sql, originalSql);
  const blocker = mode === "none" ? null : saveBlocker({ mode, sql, originalSql, phase, name });
  // What stops Save outright: anything but a missing name, which is reported
  // when Save is pressed instead of greying the button out for it.
  const runBlocker = blocker !== null && blocker !== sourceNameProblem(name) ? blocker : null;
  // A chart is always drawn from the SAVED source, so while the SQL is new or
  // differs from the saved one the live chart preview cannot show it.
  const holdsChartPreview = mode === "new" || changed;

  const { reset: resetAct } = act;
  const cancel = React.useCallback(() => {
    runSeq.current += 1;
    resetAct();
    setSql(""); setTypedName(null); setRanSql(null); setOriginal(null); setSaveAttempted(false); setMode("none");
  }, [resetAct]);
  function startNew() {
    cancel(); setMode("new");
  }
  function startEdit(source: SqlSource) {
    cancel(); setOriginal(source); setSql(source.sql); setUsage({ kind: "loading" }); setMode("edit");
  }

  async function run() {
    if (!hasStatement(sql) || act.status === "pending") return;
    const text = sql;
    const seq = ++runSeq.current;
    setRanSql(text);
    const result = await act.run(text);
    // A run started after this one, or a cancelled draft, owns the pickers now.
    if (result && seq === runSeq.current) onColumnsRef.current(result.columns);
  }

  // The count comes from the full stored-chart list (every dashboard), the
  // same list the API's delete guard counts from. If it cannot be read, no
  // number is shown rather than a guess.
  const editedId = mode === "edit" ? original?.id : undefined;
  React.useEffect(() => {
    if (!editedId) return;
    const controller = new AbortController();
    apiFetch("/api/dashboard/specs", { cache: "no-store", signal: controller.signal })
      .then(async (res) => {
        if (!res.ok) throw new Error("charts not listed");
        const json = (await res.json()) as { charts?: unknown };
        if (!Array.isArray(json.charts)) throw new Error("charts not listed");
        setUsage({ kind: "known", count: otherChartsUsing(json.charts, editedId, editChartId) });
      })
      .catch(() => { if (!controller.signal.aborted) setUsage({ kind: "unknown" }); });
    return () => controller.abort();
  }, [editedId, editChartId]);

  return {
    mode, original, sql, setSql, name, setName: setTypedName,
    nameFollowsChart: typedName === null && mode === "new",
    showNameError: typedName !== null || saveAttempted,
    markSaveAttempted: () => setSaveAttempted(true),
    phase, running: act.status === "pending",
    result: act.data, runError: phase === "failed" ? act.error?.message ?? null : null,
    changed, blocker, runBlocker, holdsChartPreview, usage,
    startNew, startEdit, cancel, run,
  };
}

export type SqlSourceDraft = ReturnType<typeof useSqlSourceDraft>;
