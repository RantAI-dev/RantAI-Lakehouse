"use client"

import { CheckBadge } from "@/components/patterns/status-badge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { governanceService } from "@/services"

/**
 * A rule's last 50 runs, newest first, each with the threshold it ran with
 * (`DATA-14`, F2 and F7). Runs are kept when a rule is edited, so a threshold
 * that differs from the rule's present one is marked "earlier": the result
 * was a verdict on what the rule used to say.
 */
export function RuleRuns({ ruleId, threshold }: { ruleId: string; threshold: string }) {
  const state = useService((signal) => governanceService.listQualityRuns(ruleId, signal), [ruleId])

  if (state.status === "loading") {
    return <p className="text-sm text-muted-foreground">Loading run history…</p>
  }
  if (state.status === "error") {
    return (
      <p className="text-sm text-destructive" role="alert">
        The run history could not be loaded: {state.error.message}
      </p>
    )
  }
  if (state.data.length === 0) {
    return <p className="text-sm text-muted-foreground">This rule has not been run yet.</p>
  }
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="text-xs">When</TableHead>
            <TableHead className="text-xs">Result</TableHead>
            <TableHead className="text-xs">Threshold it ran with</TableHead>
            <TableHead className="text-xs">By</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {state.data.map((run) => (
            <TableRow key={`${run.at}-${run.runBy}-${run.value}`}>
              <TableCell className="py-1.5 align-top text-xs">{formatRelativeTime(run.at)}</TableCell>
              <TableCell className="py-1.5 align-top">
                <div className="flex flex-col items-start gap-0.5">
                  <CheckBadge status={run.status} />
                  <span className="text-xs text-muted-foreground">{run.value}</span>
                </div>
              </TableCell>
              <TableCell className="py-1.5 align-top text-xs">
                {run.threshold === null ? (
                  <span className="text-muted-foreground">not recorded</span>
                ) : (
                  <>
                    <span className="font-mono">{run.threshold}</span>
                    {run.threshold !== threshold ? (
                      <span className="ml-1 text-muted-foreground">(earlier)</span>
                    ) : null}
                  </>
                )}
              </TableCell>
              <TableCell className="py-1.5 align-top text-xs">{run.runBy}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}
