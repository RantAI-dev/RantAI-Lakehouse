"use client";

import * as React from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { AnimatePresence, motion } from "motion/react";
import {
  BarChart3,
  BellRing,
  Braces,
  CheckCircle,
  ChevronDown,
  Clock,
  Download,
  Database,
  GitBranch,
  Hammer,
  Library,
  ListChecks,
  Loader2,
  Plug,
  Waypoints,
  Wrench,
  XCircle,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { encodeFilters } from "@/lib/dashboard-filter-state";
import { ErrorWithReference } from "@/components/error-reference";

export type ToolStep = { tool: string; args: unknown; ok: boolean; result: unknown };

export const TOOL_LABEL: Record<string, string> = {
  run_sql: "SQL query",
  ask_user: "Ask the user",
  list_datasets: "Search datasets",
  lakehouse_overview: "Lakehouse overview",
  get_ingest_spec: "Ingest spec",
  set_ingest_spec: "Set ingest spec",
  discover_source: "Discover source tables",
  run_ingest: "Run ingest",
  list_ingest_runs: "Ingest runs",
  rotate_connector_credential: "Rotate credential",
  create_pipeline: "Create pipeline",
  get_pipeline: "Pipeline detail",
  mark_pipeline_ready: "Mark pipeline ready",
  list_iceberg_tables: "Iceberg tables",
  describe_iceberg_table: "Iceberg table",
  get_table_maintenance: "Table maintenance",
  set_table_maintenance: "Set table maintenance",
  get_capacity: "Storage capacity",
  describe_dataset: "Dataset schema",
  get_lineage: "Data lineage",
  get_quality: "Data quality",
  trigger_lakehouse_build: "Build lakehouse",
  get_build_status: "Build status",
  describe_mart: "View Gold mart",
  create_chart: "Create chart",
  update_chart: "Edit chart",
  format_chart: "Format chart",
  set_dashboard_theme: "Set dashboard theme",
  set_cache_rule: "Set how long results are remembered",
  list_charts: "List charts",
  delete_chart: "Delete chart",
  create_board: "Create board",
  list_boards: "List boards",
  suggest_dashboard: "Design dashboard",
  // AI-1 / AI-2 / AI-20 (plan 20). The fallback rule would also produce
  // "Set board view filters" / "Save board filters" etc., but the explicit
  // labels keep the wording stable and tell `set_board_view_filters` apart
  // from `save_board_filters` (the first only changes what this viewer sees,
  // the second saves the board's default).
  rename_board: "Rename board",
  delete_board: "Delete board",
  arrange_board: "Arrange board",
  move_chart: "Move chart",
  set_board_view_filters: "Set view filters",
  save_board_filters: "Save default filters",
  // BI-1 / AI-10: the link opens the same download the dashboard's menu makes.
  export_dashboard: "Export dashboard",
  export_chart: "Export chart data",
  // RPT-1 / AI-10: scheduled reports by email.
  create_report_subscription: "Schedule report",
  list_report_subscriptions: "List scheduled reports",
  update_report_subscription: "Change scheduled report",
  delete_report_subscription: "Delete scheduled report",
};

/**
 * The only addresses an export step may link to: the console's own download
 * routes (`BI-1`). A result is data from the server, but the card never turns
 * an arbitrary string into a link.
 */
const EXPORT_LINK = /^\/api\/dashboard\/(export\/pdf|chart-export)\?/;

/** A tool result that is a download link (`export_dashboard`, `export_chart`), or null. */
export function exportLink(tool: string, result: unknown): { url: string; fileName: string } | null {
  if (tool !== "export_dashboard" && tool !== "export_chart") return null;
  const r = asObj(result);
  if (typeof r.url !== "string" || !EXPORT_LINK.test(r.url)) return null;
  return { url: r.url, fileName: typeof r.fileName === "string" ? r.fileName : "download" };
}

/**
 * The board the dashboard page is showing, extracted from `/dashboards/<id>`.
 * Returns `null` for any other path (e.g. the assistant panel's edge cases).
 * Exported so its own tests can pin the parse directly: every rule that
 * decides whether a step's board is the one on screen rests on it.
 */
export function boardFromPath(pathname: string | null): string | null {
  if (!pathname) return null
  const m = /^\/dashboards\/([^/?#]+)/.exec(pathname)
  return m ? decodeURIComponent(m[1] ?? "") : null
}

export function asObj(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}

/** CSS horizontal bar chart when the result is 1 label column + 1 number column. */
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
  // AI-2 (plan 20): read `set_board_view_filters` needs the page's own URL,
  // read at the top of `StepBody` so a conditional return above does not
  // make the rule of hooks complain.
  const here = usePathname();

  // Confirm/Cancel live on the message's confirmation card (`chat-messages`),
  // which also recovers a title the model left empty; a second pair of
  // buttons here only asked the same question twice.
  if (res.needs_confirmation) {
    return (
      <p className="mt-1 text-[11px] text-muted-foreground">
        Waiting for your confirmation below.
      </p>
    );
  }

  if (res.cancelled) {
    return <p className="mt-1 text-[11px] text-muted-foreground">Cancelled.</p>;
  }

  if (res.needs_approval) {
    const approvalId = String(res.approval_id ?? "");
    return (
      <div className="mt-1 space-y-2 rounded border border-amber-500/30 bg-amber-500/10 px-2 py-1.5 text-[11px]">
        <p className="text-foreground">{String(res.summary ?? "This action requires approval.")}</p>
        <Link
          href={approvalId ? `/agents/approvals?id=${encodeURIComponent(approvalId)}` : "/agents/approvals"}
          className="inline-flex items-center gap-1 font-medium text-amber-700 underline underline-offset-2 dark:text-amber-400"
        >
          View in Approvals →
        </Link>
      </div>
    );
  }

  if ("error" in res) {
    return (
      <p className="mt-1 text-[11px] text-destructive">
        <ErrorWithReference
          message={String(res.error)}
          errorId={typeof res.errorId === "string" ? res.errorId : undefined}
        />
      </p>
    );
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
        {typeof res.note === "string" ? <p className="mt-1 text-[11px] text-muted-foreground">{res.note}</p> : null}
      </div>
    );
  }

  if (step.tool === "get_quality" && Array.isArray(res.summary)) {
    return <QualityChips summary={res.summary as { verdict: string; n: string }[]} />;
  }

  if (step.tool === "list_datasets" && Array.isArray(res.datasets)) {
    // `sourceKind` replaced `tier` (primer/sekunder is where data comes
    // from, not a layer); `tier` is still read for sessions saved before.
    const ds = res.datasets as { slug: string; title: string; sourceKind?: string; tier?: string }[];
    return (
      <ul className="mt-1 space-y-0.5 text-[11px]">
        {ds.slice(0, 10).map((d) => (
          <li key={d.slug} className="truncate">
            <span className="text-muted-foreground">[{d.sourceKind ?? d.tier}]</span> {d.title}
          </li>
        ))}
        {ds.length > 10 ? <li className="text-muted-foreground">+{ds.length - 10} more…</li> : null}
      </ul>
    );
  }

  if (step.tool === "lakehouse_overview") {
    const layer = (key: string) => asObj(res[key]);
    const count = (v: unknown) => (Array.isArray(v) ? v.length : 0);
    return (
      <ul className="mt-1 space-y-0.5 text-[11px]">
        <li><span className="font-medium">Bronze</span> <span className="text-muted-foreground">· {count(layer("bronze").datasets)} datasets</span></li>
        <li><span className="font-medium">Silver</span> <span className="text-muted-foreground">· {count(layer("silver").tables)} tables</span></li>
        <li><span className="font-medium">Gold</span> <span className="text-muted-foreground">· {count(layer("gold").tables)} tables</span></li>
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
    const marts = Array.isArray(res.marts) ? (res.marts as { mart: string; rows: number | null }[]) : null;
    if (marts) {
      return (
        <ul className="mt-1 space-y-0.5 text-[11px]">
          {marts.map((m) => (
            <li key={m.mart} className="truncate"><span className="font-mono">{m.mart}</span> <span className="text-muted-foreground">· {m.rows === null ? "row count unknown" : `${m.rows.toLocaleString("id-ID")} rows`}</span></li>
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

  // AI-2 (plan 20): when the assistant applied filters to a board the page
  // already shows, say so on the step card itself; when it applied them to
  // another board, link to it with the filters already in the address (built
  // with the same `?f=` helper the chip editor uses).
  if (step.tool === "set_board_view_filters") {
    const argBoard = typeof asObj(step.args).board === "string" ? (asObj(step.args).board as string) : "";
    const resultFilters = Array.isArray(res.filters) ? (res.filters as unknown[]) : [];
    const onScreen = argBoard && here && boardFromPath(here) === argBoard;
    if (onScreen) {
      return (
        <p className="mt-1 text-[11px] text-emerald-600 dark:text-emerald-400">
          ✓ Filters applied
        </p>
      );
    }
    if (argBoard) {
      const enc = encodeFilters(resultFilters as Parameters<typeof encodeFilters>[0]);
      const href = enc ? `/dashboards/${encodeURIComponent(argBoard)}?f=${enc}` : `/dashboards/${encodeURIComponent(argBoard)}`;
      return (
        <p className="mt-1 text-[11px] text-muted-foreground">
          <Link href={href} className="font-medium text-foreground underline underline-offset-2">
            Open {argBoard}
          </Link>{" "}
          with these filters applied.
        </p>
      );
    }
  }

  // Default: a compact JSON dump.
  return (
    <pre className="mt-1 overflow-x-auto rounded bg-muted/60 px-2 py-1.5 font-mono text-[10px] text-muted-foreground">
      {JSON.stringify(step.result, null, 2).slice(0, 600)}
    </pre>
  );
}

/**
 * The Download button of an export result (BI-1 QA fix A1). A plain link to the
 * console's own download route: the browser sends the person's session with it,
 * so the file is made when they open it, with their permissions (AI-10, D9).
 */
function ExportDownload({ link }: { link: { url: string; fileName: string } }) {
  return (
    <div className="mb-1 ml-7 flex flex-wrap items-center gap-2 text-[11px]">
      <a
        href={link.url}
        download={link.fileName}
        className="inline-flex items-center gap-1.5 rounded-md border border-border bg-background px-2.5 py-1 font-medium text-foreground hover:bg-muted"
      >
        <Download className="size-3.5" aria-hidden /> Download {link.fileName}
      </a>
      <span className="text-muted-foreground">Made when you open it, with your permissions.</span>
    </div>
  );
}

/** Icon per tool family, as in the RantAI-Agents tool indicator. */
function ToolIcon({ tool }: { tool: string }) {
  const cls = "size-3 shrink-0";
  if (tool === "run_sql" || tool.includes("query")) return <Database className={cls} aria-hidden />;
  if (tool.includes("dataset") || tool.includes("mart")) return <Library className={cls} aria-hidden />;
  if (tool.includes("lineage")) return <Waypoints className={cls} aria-hidden />;
  if (tool.includes("quality")) return <ListChecks className={cls} aria-hidden />;
  if (tool.includes("chart") || tool.includes("board") || tool.includes("dashboard")) return <BarChart3 className={cls} aria-hidden />;
  if (tool.includes("build") || tool.includes("maintenance")) return <Hammer className={cls} aria-hidden />;
  if (tool.includes("pipeline")) return <GitBranch className={cls} aria-hidden />;
  if (tool.includes("connector")) return <Plug className={cls} aria-hidden />;
  if (tool.includes("alert")) return <BellRing className={cls} aria-hidden />;
  return <Wrench className={cls} aria-hidden />;
}

/** "Search datasets" for a known tool, "list pipelines" style for the rest. */
export function toolLabel(tool: string): string {
  return TOOL_LABEL[tool] ?? tool.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase());
}

/** The most telling argument, quoted after the tool name (the Agents indicator's input summary). */
function inputSummary(args: unknown): string | null {
  const a = asObj(args);
  for (const key of ["sql", "query", "question", "title", "name", "slug", "mart", "dataset", "id"]) {
    const v = a[key];
    if (typeof v === "string" && v.trim()) return v.replace(/\s+/g, " ").trim();
  }
  return null;
}

/** First word bold, the rest plain: "**Search** datasets". */
function ToolName({ label }: { label: string }) {
  const [head, ...rest] = label.split(" ");
  return (
    <span>
      <span className="font-medium text-foreground">{head}</span> {rest.join(" ")}
    </span>
  );
}

/** A tool Copilot is running right now (live, before its result exists). */
export function RunningToolRow({ tool }: { tool: string }) {
  return (
    <div className="flex items-center gap-2 py-1 text-xs text-muted-foreground" role="status">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-lg bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)]">
        <Loader2 className="size-3 animate-spin text-[var(--brand-1)]" aria-hidden />
      </span>
      <ToolIcon tool={tool} />
      <span>
        {toolLabel(tool)}
        <span className="motion-safe:animate-pulse">…</span>
      </span>
    </div>
  );
}

/** A tool Copilot already ran, live (no result yet on this client) — a check and its name. */
export function FinishedToolRow({ tool }: { tool: string }) {
  return (
    <div className="flex items-center gap-2 py-1 text-xs text-muted-foreground">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-lg bg-emerald-500/10">
        <CheckCircle className="size-3 text-emerald-500" aria-hidden />
      </span>
      <ToolIcon tool={tool} />
      <ToolName label={toolLabel(tool)} />
    </div>
  );
}

/**
 * One tool call in an answer — the RantAI-Agents tool indicator: a status
 * tile (done, failed, or waiting on the user), the tool's name and its
 * main input, and a disclosure with the result (`StepBody`) and the raw
 * JSON. Collapsed by default so the answer leads; a step that waits on
 * approval or confirmation starts open.
 */
export function ToolStepCard({ step }: { step: ToolStep }) {
  const tool = step.tool;
  const res = asObj(step.result);
  const needsApproval = Boolean(res.needs_approval);
  const pending = Boolean(res.needs_confirmation) || needsApproval;
  const hasError = !step.ok || "error" in res;
  const [open, setOpen] = React.useState(pending);
  const [raw, setRaw] = React.useState(false);
  const summary = inputSummary(step.args);
  // BI-1 QA fix A1: an export result is a Download button built from the tool's own
  // `url`, shown on the card itself (not inside the collapsed details), never from
  // text the model wrote.
  const download = exportLink(tool, step.result);
  return (
    <div className="my-0.5">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className={cn(
          "group/tool flex w-full items-center gap-2 rounded-md py-1 text-left text-xs transition-colors",
          hasError ? "text-destructive" : "text-muted-foreground hover:text-foreground",
        )}
      >
        {pending ? (
          <span className="flex size-5 shrink-0 items-center justify-center rounded-lg bg-amber-500/15">
            <Clock className="size-3 text-amber-600 dark:text-amber-400" aria-hidden />
          </span>
        ) : hasError ? (
          <span className="flex size-5 shrink-0 items-center justify-center rounded-lg bg-destructive/10">
            <XCircle className="size-3 text-destructive" aria-hidden />
          </span>
        ) : (
          <span className="flex size-5 shrink-0 items-center justify-center rounded-lg bg-emerald-500/10">
            <CheckCircle className="size-3 text-emerald-500" aria-hidden />
          </span>
        )}
        <ToolIcon tool={tool} />
        <span className="flex min-w-0 flex-1 items-center gap-1.5">
          <ToolName label={toolLabel(step.tool)} />
          {summary ? (
            <span className="max-w-[260px] truncate text-foreground/70" title={summary}>
              &ldquo;{summary}&rdquo;
            </span>
          ) : null}
          {needsApproval ? (
            <span className="shrink-0 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
              pending approval
            </span>
          ) : res.needs_confirmation ? (
            <span className="shrink-0 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
              needs confirmation
            </span>
          ) : null}
        </span>
        <ChevronDown
          className={cn("size-3 shrink-0 transition-transform", !open && "-rotate-90")}
          aria-hidden
        />
      </button>
      {download ? <ExportDownload link={download} /> : null}
      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.15 }}
            className="overflow-hidden"
          >
            <div className="mt-1 ml-2.5 space-y-2 border-l-2 border-border/50 pl-3 text-xs">
              {raw ? (
                <pre className="max-h-[220px] overflow-auto rounded-lg bg-muted/50 p-2 font-mono text-[11px]">
                  {JSON.stringify({ input: step.args, output: step.result }, null, 2)}
                </pre>
              ) : (
                <StepBody step={step} />
              )}
              <button
                type="button"
                onClick={() => setRaw((r) => !r)}
                className="flex items-center gap-1 text-[10px] text-muted-foreground/70 transition-colors hover:text-muted-foreground"
              >
                <Braces className="size-2.5" aria-hidden />
                {raw ? "Hide raw" : "View raw"}
              </button>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
