"use client"

import { Button } from "@/components/ui/button"
import { sqlSummary } from "@/lib/sql-text"
import type { useQueryStudio } from "./use-query-studio"

/**
 * The result tabs of a run: one per statement, in the order they were
 * written.
 *
 * A statement that did not run says so on its tab. "Not run" is the whole
 * truth about it: the batch stopped at the failure before it, and nothing
 * was sent to the engine.
 */
export function QueryResultTabs({
  studio,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
}) {
  const { activeRuns, activeRunIndex, setActiveRunIndex } = studio

  return (
    <div
      role="group"
      aria-label="Statement results"
      className="flex flex-wrap items-center gap-1"
    >
      {activeRuns.map((run, index) => {
        const open = index === activeRunIndex
        const summary = sqlSummary(run.statement.text)
        return (
          <Button
            key={index}
            size="sm"
            variant={open ? "secondary" : "ghost"}
            aria-current={open ? "true" : undefined}
            title={summary}
            onClick={() => setActiveRunIndex(index)}
          >
            {run.status === "not-run" ? "Not run" : summary}
          </Button>
        )
      })}
    </div>
  )
}
