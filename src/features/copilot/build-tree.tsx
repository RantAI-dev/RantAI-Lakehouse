"use client";

import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Pohon pipeline lakehouse LIVE untuk mode Build. Polling /api/ai/build-status
 * dgn runId sampai run selesai; tiap step (Bronze→Silver→Gold) menghijau saat
 * berjalan. Ini yang bikin "bikin data lewat chat" kelihatan nyata.
 */

type Step = { key: string; status: string };

const STEP_LABEL: Record<string, string> = {
  bronze_files: "Bronze · berkas",
  bronze_sdi: "Bronze · SDI",
  lake_db: "Lake DB",
  functions_dim: "Silver · fungsi & dimensi",
  silver_auto: "Silver · auto-type",
  quality_gate: "Quality gate",
  curated_gold: "Gold · kurasi",
  gold_iceberg: "Gold · Iceberg",
};
const ORDER = Object.keys(STEP_LABEL);

const TERMINAL = new Set(["SUCCESS", "FAILURE", "CANCELED"]);

function dot(status: string) {
  switch (status) {
    case "SUCCESS":
      return "bg-emerald-500";
    case "IN_PROGRESS":
      return "bg-sky-500 animate-pulse";
    case "FAILURE":
      return "bg-red-500";
    case "SKIPPED":
      return "bg-muted-foreground/40";
    default:
      return "bg-muted-foreground/30";
  }
}

export function BuildTree({ runId }: { runId: string }) {
  const [status, setStatus] = React.useState<string>("STARTING");
  const [steps, setSteps] = React.useState<Step[]>([]);

  React.useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;

    async function tick() {
      try {
        const res = await fetch(`/api/ai/build-status?runId=${encodeURIComponent(runId)}`, { cache: "no-store" });
        const json = await res.json();
        if (!alive) return;
        if (json.status) setStatus(json.status);
        if (Array.isArray(json.steps)) setSteps(json.steps);
        if (!TERMINAL.has(json.status)) timer = setTimeout(tick, 3000);
      } catch {
        if (alive) timer = setTimeout(tick, 5000);
      }
    }
    tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [runId]);

  const byKey = new Map(steps.map((s) => [s.key, s.status]));
  // Urutkan step: yang dikenal ikut ORDER, sisanya di belakang.
  const known = ORDER.map((k) => ({ key: k, status: byKey.get(k) ?? "PENDING" }));
  const extra = steps.filter((s) => !STEP_LABEL[s.key]);
  const all = [...known, ...extra];

  const done = steps.filter((s) => s.status === "SUCCESS").length;
  const overall =
    status === "SUCCESS" ? "Selesai" : status === "FAILURE" ? "Gagal" : status === "CANCELED" ? "Dibatalkan" : "Berjalan…";
  const overallTone =
    status === "SUCCESS"
      ? "text-emerald-600 dark:text-emerald-400"
      : status === "FAILURE"
        ? "text-red-600 dark:text-red-400"
        : "text-sky-600 dark:text-sky-400";

  return (
    <div className="mt-2 rounded-md border border-border bg-background/60 p-2.5">
      <div className="mb-2 flex items-center gap-2 text-xs">
        <span className="font-medium">Pipeline lakehouse</span>
        <span className={cn("font-medium", overallTone)}>{overall}</span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground">
          {done}/{ORDER.length}
        </span>
      </div>
      <ol className="space-y-1">
        {all.map((s) => (
          <li key={s.key} className="flex items-center gap-2 text-[11px]">
            <span className={cn("inline-block h-2 w-2 rounded-full", dot(s.status))} />
            <span className={cn(s.status === "SUCCESS" ? "text-foreground" : "text-muted-foreground")}>
              {STEP_LABEL[s.key] ?? s.key}
            </span>
            {s.status === "IN_PROGRESS" ? <span className="text-[10px] text-sky-500">berjalan</span> : null}
            {s.status === "FAILURE" ? <span className="text-[10px] text-red-500">gagal</span> : null}
          </li>
        ))}
      </ol>
      <p className="mt-2 font-mono text-[9px] text-muted-foreground">run {runId.slice(0, 8)}</p>
    </div>
  );
}
