"use client";

import * as React from "react";
import { cn } from "@/lib/utils";

export type ToolStep = { tool: string; args: unknown; ok: boolean; result: unknown };

const TOOL_LABEL: Record<string, string> = {
  run_sql: "SQL query",
  list_datasets: "Search datasets",
  describe_dataset: "Dataset schema",
  get_lineage: "Data lineage",
  get_quality: "Data quality",
  trigger_lakehouse_build: "Build lakehouse",
  get_build_status: "Build status",
  describe_mart: "View Gold mart",
  create_chart: "Create chart",
  update_chart: "Edit chart",
  list_charts: "List charts",
  delete_chart: "Delete chart",
  create_board: "Create board",
  list_boards: "List boards",
  suggest_dashboard: "Design dashboard",
};

function asObj(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}

/** Bar-chart horizontal CSS bila hasil = 1 kolom label + 1 kolom angka. */
function MiniBar({ columns, rows }: { columns: string[]; rows: Record<string, unknown>[] }) {
  if (columns.length < 2 || rows.length === 0 || rows.length > 12) return null;
  const [labelCol, valCol] = columns;
  const points = rows.map((r) => ({ label: String(r[labelCol] ?? ""), val: Number(r[valCol]) }));
  if (points.some((p) => !Number.isFinite(p.val))) return null;
  const max = Math.max(...points.map((p) => Math.abs(p.val))) || 1;
  return (
    <div className="mt-2 space-y-1">
      {points.map((p, i) => (
        <div key={i} className="flex items-center gap-2 text-[11px]">
          <span className="w-28 shrink-0 truncate text-muted-foreground" title={p.label}>
            {p.label}
          </span>
          <div className="h-3 flex-1 rounded-sm bg-muted/50">
            <div className="h-3 rounded-sm bg-primary/70" style={{ width: `${Math.max(2, (Math.abs(p.val) / max) * 100)}%` }} />
          </div>
          <span className="w-24 shrink-0 text-right tabular-nums">{p.val.toLocaleString("id-ID")}</span>
        </div>
      ))}
    </div>
  );
}

function ResultTable({ columns, rows }: { columns: string[]; rows: Record<string, unknown>[] }) {
  const shown = rows.slice(0, 8);
  return (
    <div className="mt-1 overflow-x-auto rounded border border-border">
      <table className="w-full border-collapse text-[11px]">
        <thead>
          <tr className="border-b border-border bg-muted/40">
            {columns.map((c) => (
              <th key={c} className="px-2 py-1 text-left font-medium text-muted-foreground">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {shown.map((r, i) => (
            <tr key={i} className="border-b border-border/40 last:border-0">
              {columns.map((c) => (
                <td key={c} className="px-2 py-1 tabular-nums">
                  {String(r[c] ?? "")}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length > shown.length ? (
        <p className="px-2 py-1 text-[10px] text-muted-foreground">+{rows.length - shown.length} more rows…</p>
      ) : null}
    </div>
  );
}

function QualityChips({ summary }: { summary: { verdict: string; n: string }[] }) {
  const tone: Record<string, string> = {
    pass: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
    ok: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
    warn: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
    fail: "bg-red-500/15 text-red-600 dark:text-red-400",
    karantina: "bg-red-500/15 text-red-600 dark:text-red-400",
  };
  return (
    <div className="mt-1 flex flex-wrap gap-1.5">
      {summary.map((s, i) => (
        <span key={i} className={cn("rounded-full px-2 py-0.5 text-[11px] font-medium", tone[s.verdict.toLowerCase()] ?? "bg-muted text-muted-foreground")}>
          {s.verdict}: {s.n}
        </span>
      ))}
    </div>
  );
}

function StepBody({ step }: { step: ToolStep }) {
  const res = asObj(step.result);
  if ("error" in res) {
    return <p className="mt-1 text-[11px] text-destructive">{String(res.error)}</p>;
  }

  if (step.tool === "run_sql") {
    const sql = String(asObj(step.args).sql ?? "");
    const columns = Array.isArray(res.columns) ? (res.columns as string[]) : [];
    const rows = Array.isArray(res.rows) ? (res.rows as Record<string, unknown>[]) : [];
    return (
      <div>
        {sql ? (
          <pre className="mt-1 overflow-x-auto rounded bg-muted/60 px-2 py-1.5 font-mono text-[10px] leading-snug text-muted-foreground">{sql}</pre>
        ) : null}
        {columns.length ? <ResultTable columns={columns} rows={rows} /> : null}
        {columns.length ? <MiniBar columns={columns} rows={rows} /> : null}
      </div>
    );
  }

  if (step.tool === "get_quality" && Array.isArray(res.summary)) {
    return <QualityChips summary={res.summary as { verdict: string; n: string }[]} />;
  }

  if (step.tool === "list_datasets" && Array.isArray(res.datasets)) {
    const ds = res.datasets as { slug: string; title: string; tier: string }[];
    return (
      <ul className="mt-1 space-y-0.5 text-[11px]">
        {ds.slice(0, 10).map((d) => (
          <li key={d.slug} className="truncate">
            <span className="text-muted-foreground">[{d.tier}]</span> {d.title}
          </li>
        ))}
        {ds.length > 10 ? <li className="text-muted-foreground">+{ds.length - 10} lain…</li> : null}
      </ul>
    );
  }

  if (step.tool === "get_lineage" && typeof res.chain === "string") {
    return <p className="mt-1 font-mono text-[11px] text-muted-foreground">{res.chain}</p>;
  }

  if (step.tool === "create_chart" && res.created) {
    return (
      <div className="mt-1 flex flex-wrap items-center gap-2 text-[11px]">
        <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 font-medium text-emerald-600 dark:text-emerald-400">
          ✓ chart created
        </span>
        <span className="font-medium text-foreground">{String(res.title ?? "")}</span>
        <span className="text-muted-foreground">{String(res.kind ?? "")} · {String(res.mart ?? "")}</span>
      </div>
    );
  }

  if (step.tool === "describe_mart") {
    const dims = Array.isArray(res.dimensions) ? (res.dimensions as string[]) : null;
    const meas = Array.isArray(res.measures) ? (res.measures as string[]) : null;
    const marts = Array.isArray(res.marts) ? (res.marts as { mart: string; rows: number }[]) : null;
    if (marts) {
      return (
        <ul className="mt-1 space-y-0.5 text-[11px]">
          {marts.map((m) => (
            <li key={m.mart} className="truncate"><span className="font-mono">{m.mart}</span> <span className="text-muted-foreground">· {m.rows.toLocaleString("id-ID")} rows</span></li>
          ))}
        </ul>
      );
    }
    if (dims || meas) {
      return (
        <div className="mt-1 space-y-1 text-[11px]">
          <p><span className="text-muted-foreground">Dimensions:</span> {(dims ?? []).join(", ") || "—"}</p>
          <p><span className="text-muted-foreground">Measures:</span> {(meas ?? []).join(", ") || "—"}</p>
        </div>
      );
    }
  }

  // Default: JSON ringkas.
  return (
    <pre className="mt-1 overflow-x-auto rounded bg-muted/60 px-2 py-1.5 font-mono text-[10px] text-muted-foreground">
      {JSON.stringify(step.result, null, 2).slice(0, 600)}
    </pre>
  );
}

export function ToolStepCard({ step }: { step: ToolStep }) {
  const [open, setOpen] = React.useState(step.tool === "run_sql");
  const label = TOOL_LABEL[step.tool] ?? step.tool;
  return (
    <div className="rounded-md border border-border bg-background/60">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs"
      >
        <span className={cn("inline-block h-1.5 w-1.5 shrink-0 rounded-full", step.ok ? "bg-emerald-500" : "bg-red-500")} />
        <span className="font-medium">{label}</span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">{open ? "−" : "+"}</span>
      </button>
      {open ? <div className="border-t border-border px-2.5 pb-2 pt-1">{<StepBody step={step} />}</div> : null}
    </div>
  );
}
