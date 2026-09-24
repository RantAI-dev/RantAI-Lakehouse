"use client"

import { SectionCard } from "@/components/patterns/section-card"
import { RowsTable } from "@/components/patterns/rows-table"
import { formatNumber } from "@/lib/format"
import type { AgentQueryResult } from "@/services/clients/agent-client"

/** How many rows of the agent's answer are previewed inline. */
const PREVIEW_ROWS = 10

/**
 * The agent's answer: what it concluded, how it got there, and a peek at
 * the rows behind it. The full result is one click away — the final SQL is
 * already loaded in the editor, ready to run.
 */
export function AgentResultCard({ result }: { readonly result: AgentQueryResult }) {
  return (
    <SectionCard title="Agent answer">
      <p className="text-sm">{result.answer}</p>
      <p className="mt-2 text-xs text-muted-foreground">
        {formatNumber(result.rowCount)} rows · the final SQL is loaded in the
        SQL tab.
      </p>
      <details className="mt-3">
        <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
          Agent steps ({result.steps.length})
        </summary>
        <ol className="mt-2 space-y-1 text-xs text-muted-foreground">
          {result.steps.map((step, i) => (
            <li key={`${step.step}-${i}`}>
              <span className="font-mono text-foreground">{step.step}</span>:{" "}
              {step.detail}
            </li>
          ))}
        </ol>
      </details>
      {result.rows.length ? (
        <div className="mt-3 space-y-1">
          <RowsTable
            columns={result.columns}
            rows={result.rows}
            maxRows={PREVIEW_ROWS}
            className="max-h-72 overflow-auto rounded-md border border-border"
          />
          {result.rows.length > PREVIEW_ROWS ? (
            <p className="text-[11px] text-muted-foreground">
              Showing {PREVIEW_ROWS} of {formatNumber(result.rows.length)} rows.
              Run the SQL for all of them.
            </p>
          ) : null}
        </div>
      ) : null}
    </SectionCard>
  )
}
