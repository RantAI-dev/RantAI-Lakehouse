"use client"

import Link from "next/link"

import { Skeleton } from "@/components/ui/skeleton"
import { formatRelativeTime } from "@/lib/format"
import { formatUntil, type PipelineRow } from "@/lib/home-pipelines"
import type { CheckState } from "@/lib/home-status"
import { cn } from "@/lib/utils"
import { SectionTitle } from "./home-ui"

const RUN_DOT: Record<string, string> = {
  failed: "bg-destructive",
  degraded: "bg-amber-500",
  completed: "bg-emerald-500",
  running: "bg-sky-500",
}

/** When each pipeline last ran, a line each (`pipelineRows`). */
export function PipelineRuns({
  state,
  rows,
}: {
  state: CheckState
  rows: PipelineRow[]
}) {
  return (
    <section className="flex min-w-0 flex-col gap-3">
      <div className="flex items-baseline justify-between gap-3 px-1">
        <SectionTitle>Pipeline runs</SectionTitle>
        <Link
          href="/pipelines"
          className="shrink-0 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        >
          All pipelines
        </Link>
      </div>
      <div className="rounded-xl border border-border bg-card p-2">
        {state === "loading" ? (
          <div className="flex flex-col gap-2 p-2">
            <Skeleton className="h-5 w-full" />
            <Skeleton className="h-5 w-4/5" />
          </div>
        ) : state === "error" ? (
          <p className="px-2 py-2 text-xs text-muted-foreground">
            Couldn&apos;t load pipelines.
          </p>
        ) : rows.length === 0 ? (
          <p className="px-2 py-2 text-xs text-muted-foreground">
            No pipeline has run or is scheduled.
          </p>
        ) : (
          <ul className="flex flex-col">
            {rows.map((r) => {
              const next = formatUntil(r.nextRunAt)
              // The row shows one time so the name keeps its room in the
              // narrow column; the whole sentence is the tooltip.
              const last = r.lastRunAt
                ? `${r.status === "failed" ? "failed" : "ran"} ${formatRelativeTime(r.lastRunAt)}`
                : "not run yet"
              return (
                <li key={r.id}>
                  <Link
                    href={`/pipelines/${encodeURIComponent(r.id)}`}
                    title={`${r.name}: ${last}${next ? `, next ${next}` : ""}`}
                    className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
                  >
                    <span
                      className={cn(
                        "size-2 shrink-0 rounded-full",
                        RUN_DOT[r.status] ?? "bg-muted-foreground/40",
                      )}
                    />
                    <span className="sr-only">{r.status}</span>
                    <span className="min-w-0 flex-1 truncate">{r.name}</span>
                    <span
                      className={cn(
                        "shrink-0 text-xs tabular-nums",
                        r.status === "failed"
                          ? "text-destructive"
                          : "text-muted-foreground",
                      )}
                    >
                      {r.lastRunAt
                        ? r.status === "failed"
                          ? last
                          : formatRelativeTime(r.lastRunAt)
                        : next
                          ? `next ${next}`
                          : "not run yet"}
                    </span>
                  </Link>
                </li>
              )
            })}
          </ul>
        )}
      </div>
    </section>
  )
}
