"use client"

import { governedPairLabels } from "@/features/catalog/governed-tags"

import type { ReactNode } from "react"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { CertificationBadge, Pill, StatusBadge } from "@/components/patterns/status-badge"
import { Skeleton } from "@/components/ui/skeleton"
import { DeprecationNotice } from "@/features/catalog/asset-certification"
import type { ActionState } from "@/hooks/use-service"
import { formatBytes } from "@/lib/format"
import { WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type { QueryEstimate, QueryPlanStage } from "@/services/contracts/queries"

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-2">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="text-right">{children}</dd>
    </div>
  )
}

/**
 * The tables a query reads, each with its certification mark (`DATA-12`
 * part 3), and a warning under the list for each deprecated one. A deprecated
 * table still runs: the warning names it and what to use instead, and never
 * blocks the query. `marks` is absent when no source is marked.
 */
function SourcesList({
  sources,
  marks,
}: {
  sources: string[]
  marks: QueryEstimate["sourceMarks"]
}) {
  return (
    <>
      <ul className="space-y-0.5">
        {sources.map((source) => {
          const mark = marks?.[source]
          const governed = governedPairLabels(mark?.governedTags)
          const tags = mark?.tags ?? []
          return (
            <li key={source} className="font-mono text-xs flex items-center flex-wrap gap-1">
              <span>{source}</span>
              {/* PR #106 review R1 BLOCKER 2: mark, governed tags, free tags */}
              <CertificationBadge
                certification={mark?.certification}
                className="align-middle font-sans"
              />
              {governed.map((pair) => (
                <Pill key={`governed:${pair}`} tone="info" className="align-middle font-sans text-[10px]">
                  {pair}
                </Pill>
              ))}
              {tags.map((tag) => (
                <Pill key={tag} tone="neutral" className="align-middle font-sans text-[10px]">
                  {tag}
                </Pill>
              ))}
            </li>
          )
        })}
      </ul>
      <div className="mt-2 space-y-1.5">
        {sources.map((source) => {
          const mark = marks?.[source]
          if (mark?.certification !== "deprecated") return null
          return (
            <div key={source} className="text-left">
              <span className="sr-only">{source}</span>
              <DeprecationNotice asset={mark} />
            </div>
          )
        })}
      </div>
    </>
  )
}

/**
 * Simple federated / multi-source plan — product language first.
 *
 * `plan` is `null` when no real plan was computed (both `/api/query/run` and
 * `/api/query/estimate` currently emit `null` here — see WS1 task 1.6). Null
 * renders an honest "not measured" line rather than an empty list, which
 * would read as "we checked and there were no stages."
 */
export function QueryPlanPanel({ plan }: { plan: QueryPlanStage[] | null }) {
  if (plan === null) {
    return (
      <div className="mt-3 space-y-1">
        <p className="text-xs font-medium text-muted-foreground">Execution plan</p>
        <p className="text-xs text-muted-foreground">Not measured.</p>
      </div>
    )
  }
  if (plan.length === 0) return null
  return (
    <div className="mt-3 space-y-2">
      <p className="text-xs font-medium text-muted-foreground">Execution plan</p>
      <ol className="space-y-2 border-l border-border pl-3">
        {plan.map((stage) => (
          <li key={stage.id} className="relative text-sm">
            <span className="absolute -left-[17px] top-1.5 size-2 rounded-full bg-muted-foreground/50" />
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{stage.label}</span>
              {stage.status ? <StatusBadge status={stage.status} /> : null}
            </div>
            <p className="text-xs text-muted-foreground">
              {stage.location} · {stage.operation}
              {stage.estimatedBytes != null
                ? ` · ~${formatBytes(stage.estimatedBytes)}`
                : ""}
            </p>
          </li>
        ))}
      </ol>
    </div>
  )
}

/**
 * The Plan tab of Query Studio's side panel (`UX-9`): the details of the
 * pre-run estimate and how the engine will run the statement. It replaced
 * the "Execution transparency" card. The engine and the estimate itself
 * (scan · cost) are the status bar's (`studio-status-bar.tsx`) and are not
 * repeated here; this tab holds the rest, and every reason the line under
 * the editor only names.
 */
export function QueryTransparencyPanel({
  state,
  estimatable,
  estimateSupported = true,
  waitingFor = null,
  onRetry,
}: {
  state: ActionState<QueryEstimate>
  /** Whether the editor holds a statement worth estimating. */
  estimatable?: boolean
  /**
   * Whether the open tab's engine has an estimate at all. Only ClickHouse
   * does: the server's `EXPLAIN ESTIMATE` has no Trino counterpart, so a Trino
   * tab never asks and the panel says so (LH-QS-4-08).
   */
  estimateSupported?: boolean
  /**
   * The first parameter the estimate waits for, or `null`. The panel names it
   * in the spelling of the run's refusal (QS-5-AC14).
   */
  waitingFor?: string | null
  onRetry: () => void
}) {
  if (!estimateSupported) {
    // Every other state is about an estimate this tab cannot have, so none
    // of them is shown: a leftover refusal or error would blame the query
    // for something the engine simply does not do.
    return (
      <PanelNote
        title="No pre-run estimate for Trino queries."
        line="Switch the engine to ClickHouse for an estimate, or run the query and read its metrics."
      />
    )
  }
  const ready =
    state.data && !state.data.error && state.data.supported !== false && estimatable !== false
  return (
    <div className="space-y-4 p-3 text-sm">
      {/* An empty editor gets no estimate and says so, rather than
          reporting a confident "0 B · 1.00 cu · Fresh · 0 s" about
          nothing. A statement that waits for a parameter names it. */}
      {waitingFor !== null ? (
        <p className="text-sm text-muted-foreground">
          {`Fill in {{${waitingFor}}} for a pre-run estimate.`}
        </p>
      ) : state.status === "idle" || estimatable === false ? (
        <p className="text-sm text-muted-foreground">Type SQL for a pre-run estimate.</p>
      ) : null}
      {state.status === "error" ? (
        <p className="text-sm text-muted-foreground">
          Estimate unavailable.{" "}
          <button
            type="button"
            onClick={onRetry}
            className="text-primary underline-offset-4 hover:underline"
          >
            Retry
          </button>
        </p>
      ) : null}
      {state.status === "pending" && !state.data ? (
        <div className="space-y-2" role="status" aria-label="Estimating">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-3/4" />
          <Skeleton className="h-4 w-5/6" />
        </div>
      ) : null}
      {/* QA LH-QS-4-08 (QS-6), decision 7: a Trino statement has no pre-run
          estimate. The server says so in a fixed sentence; the panel shows
          no source and no error, because nothing was sent and nothing was
          read. The engine is the status bar's. */}
      {state.data?.supported === false && estimatable !== false ? (
        <div className="space-y-1">
          <p className="text-sm font-medium">No pre-run estimate</p>
          <p className="text-sm text-muted-foreground">{state.data.reason}</p>
        </div>
      ) : null}
      {/* A query ClickHouse cannot even plan has no estimate. Its reason is
          the server's fixed sentence and reference id (the engine's text is
          logged under that reference, `SEC-11`). It is the most useful thing
          on this panel, so it is shown instead of a row of zeros. */}
      {state.data?.error && state.data.supported !== false && estimatable !== false ? (
        <div className="space-y-1">
          <p className="text-sm font-medium">This query cannot be planned</p>
          <p className="font-mono text-xs leading-relaxed text-muted-foreground">
            {state.data.error}
          </p>
        </div>
      ) : null}
      {ready && state.data ? (
        <dl className={state.status === "pending" ? "space-y-2 opacity-60" : "space-y-2"}>
          <Row label="Workload">
            {state.data.workloadClass ? WORKLOAD_CLASS_LABEL[state.data.workloadClass] : "—"}
          </Row>
          <Row label="Cache">
            {/* Null means nobody asked the cache, which is not the same as
                "not eligible" — saying either would be a guess. */}
            {state.data.cacheEligible == null ? (
              <span className="text-muted-foreground">Not measured</span>
            ) : state.data.cacheEligible ? (
              "Eligible"
            ) : (
              "Not eligible"
            )}
          </Row>
          <Row label="Freshness">
            {/* QS-10 (LH-QS-1-11), D2: without catalog:read the server sends no
                freshness at all — say why instead of showing a stale 0. */}
            {state.data.catalogHidden ? (
              <span className="text-muted-foreground">Needs catalog access</span>
            ) : (
              <FreshnessIndicator lagSeconds={state.data.freshnessLagSeconds} />
            )}
          </Row>
          <div>
            <dt className="text-muted-foreground">Policy obligations</dt>
            <dd className="mt-1 flex flex-wrap gap-1">
              {state.data.policyObligations == null ? (
                // QS-10 (LH-QS-2-05), D4: null now only comes from an older
                // server or a Trino estimate, which is not rewritten. "None"
                // would read as "checked, nothing applies".
                <span className="text-xs text-muted-foreground">Not evaluated</span>
              ) : state.data.policyObligations.length ? (
                state.data.policyObligations.map((o) => (
                  <Pill key={o} tone="neutral">
                    {o}
                  </Pill>
                ))
              ) : (
                <span className="text-xs text-muted-foreground">None</span>
              )}
            </dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Sources</dt>
            <dd className="mt-1">
              {/* QS-10 (LH-QS-1-11), D2/D3: withhold the catalog's answer, or
                  say the plan named no table — never invent a source. */}
              {state.data.catalogHidden ? (
                <span className="text-muted-foreground">Needs catalog access</span>
              ) : state.data.sources.length === 0 ? (
                <span className="text-muted-foreground">No table found in the plan</span>
              ) : (
                <SourcesList sources={state.data.sources} marks={state.data.sourceMarks} />
              )}
            </dd>
          </div>
          <QueryPlanPanel plan={state.data.plan} />
        </dl>
      ) : null}
    </div>
  )
}

/** A tab with nothing to show but why. */
function PanelNote({ title, line }: { title: string; line: string }) {
  return (
    <div className="grid gap-1 px-4 py-8 text-center">
      <p className="text-sm font-medium">{title}</p>
      <p className="text-xs leading-5 text-muted-foreground">{line}</p>
    </div>
  )
}
