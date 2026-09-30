"use client"

import * as React from "react"
import Link from "next/link"
import { useParams, usePathname, useRouter, useSearchParams } from "next/navigation"
import {
  CalendarClockIcon,
  EllipsisIcon,
  HandIcon,
  PauseIcon,
  PencilIcon,
  PlayIcon,
  RadarIcon,
  RotateCcwIcon,
  Trash2Icon,
} from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDateTime, formatDuration, formatRelativeTime } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import type { EntityStatus } from "@/lib/status"
import { cn } from "@/lib/utils"
import { pipelineService } from "@/services"
import type { PipelineDetail, PipelineRun } from "@/services/contracts/pipelines"
import { AuthoredFlow, DagsterDefinition } from "./pipeline-definition"
import { PipelineMasthead, PipelineVitals } from "./pipeline-detail-header"
import { isLiveRun, runDurationMs, summarizeRuns } from "./pipeline-run-stats"
import { describeCron, isScheduleRunning, localizeScheduleTime, parseSchedule } from "./pipeline-schedule"
import { RunHistoryStrip, runFill, statusLabel } from "./run-history-strip"
import { RunInspector } from "./run-inspector"
import { ScheduleHistory } from "./schedule-history"

/** Poll cadence: fast while a run is live or one was just launched, slow otherwise. */
const LIVE_POLL_MS = 4000
const IDLE_POLL_MS = 30_000
/** How long after a launch to keep polling fast: the new run may take a few seconds to be listed. */
const LAUNCH_WINDOW_MS = 20_000

type TabId = "runs" | "definition" | "settings"

function TriggerGlyph({ run }: { run: PipelineRun }) {
  const kind = run.trigger?.kind
  const Icon =
    kind === "schedule" ? CalendarClockIcon : kind === "sensor" ? RadarIcon : kind === "retry" ? RotateCcwIcon : HandIcon
  return <Icon className="size-3.5 shrink-0 text-muted-foreground" aria-label={kind ?? "manual"} />
}

/** The run list beside the inspector: newest first, retries marked with the run they repeat. */
function RunList({
  runs,
  selectedId,
  onSelect,
  now,
}: {
  runs: readonly PipelineRun[]
  selectedId: string | null
  onSelect: (id: string) => void
  now: number
}) {
  return (
    <ol className="flex max-h-[44rem] flex-col gap-0.5 overflow-y-auto pr-1" aria-label="Runs">
      {runs.map((run) => {
        const selected = run.id === selectedId
        const d = runDurationMs(run, now)
        return (
          <li key={run.id}>
            <button
              type="button"
              onClick={() => onSelect(run.id)}
              aria-current={selected ? "true" : undefined}
              className={cn(
                "group relative grid w-full grid-cols-[auto_1fr_auto] items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors",
                selected ? "bg-primary/8" : "hover:bg-muted/60"
              )}
            >
              {selected ? <span aria-hidden className="absolute inset-y-1.5 left-0 w-0.5 rounded-full bg-primary" /> : null}
              <span
                aria-hidden
                className={cn("size-2.5 rounded-full ring-4 ring-transparent", runFill(run.status), isLiveRun(run) && "animate-pulse")}
              />
              <span className="min-w-0">
                <span className="flex items-center gap-1.5 text-sm font-medium">
                  <span className="truncate">{formatRelativeTime(run.startedAt, now)}</span>
                  <TriggerGlyph run={run} />
                </span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground">
                  {run.parentRunId ? `↳ retry of ${run.parentRunId.slice(0, 8)}` : run.id.slice(0, 8)}
                </span>
              </span>
              <span className="flex flex-col items-end gap-0.5">
                <span className="font-mono text-xs tabular-nums text-muted-foreground">{d === null ? "—" : formatDuration(d)}</span>
                <span className="sr-only">{statusLabel(run.status)}</span>
              </span>
            </button>
          </li>
        )
      })}
    </ol>
  )
}

function SettingsTab({ p, scheduleText }: { p: PipelineDetail; scheduleText: string }) {
  return (
    <div className="flex flex-col gap-4">
      <section className="rounded-xl border border-border bg-card p-4">
        <h3 className="mb-3 font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">Pipeline</h3>
        <MetadataList
          columns={3}
          items={[
            { label: "Id", value: <span className="font-mono text-xs">{p.id}</span> },
            { label: "Engine", value: p.engine === "authored" ? "Authored (Postgres definition)" : "Dagster job" },
            { label: "Kind", value: p.kind },
            { label: "Owner", value: p.owner },
            { label: "Schedule", value: scheduleText },
            { label: "Next run", value: p.nextRunAt ? formatDateTime(p.nextRunAt) : "—" },
            { label: "Last run", value: p.lastRunAt ? formatDateTime(p.lastRunAt) : "—" },
            // No SLA is defined per job: `dataset_sla` is keyed by table,
            // so one job-level boolean would not be honest (routes::pipelines).
            { label: "SLA", value: p.slaOk === null ? "Not defined per pipeline" : p.slaOk ? "OK" : "Breached" },
            { label: "Freshness", value: <FreshnessIndicator lagSeconds={p.freshnessLagSeconds} /> },
          ]}
        />
      </section>
      <section className="rounded-xl border border-border bg-card p-4">
        <h3 className="mb-3 font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">Run configuration</h3>
        {p.config.length === 0 ? (
          <p className="text-sm text-muted-foreground">This pipeline has no recorded run configuration.</p>
        ) : (
          <MetadataList columns={2} items={p.config.map((c) => ({ label: c.key, value: <span className="font-mono text-xs">{c.value}</span> }))} />
        )}
      </section>
    </div>
  )
}

/**
 * `/pipelines/[pipelineId]`: one pipeline, operated.
 *
 * Built around the run, the way the Dagster, Airflow and Databricks run
 * pages are: a masthead with the trigger and schedule, four vitals
 * computed from the run history, a strip of the last 30 runs, and an
 * inspector for the selected run (the error first, then the step timeline
 * and the log). The selected run and tab live in the URL (`?run=`,
 * `?tab=`), so a link opens the same view.
 *
 * Runs poll every 4 s while one is live, or for 20 s after a launch, since
 * a new run takes a few seconds to be listed, and every 30 s otherwise.
 * Polls keep the page on screen (`keepDataOnReload`); they never flash a
 * skeleton.
 */
export function PipelineDetailPage() {
  const { pipelineId } = useParams<{ pipelineId: string }>()
  const router = useRouter()
  const pathname = usePathname()
  const search = useSearchParams()

  const state = useService((s) => pipelineService.getPipeline(pipelineId, s), [pipelineId], { keepDataOnReload: true })
  const runsState = useService((s) => pipelineService.listRuns(pipelineId, s), [pipelineId], { keepDataOnReload: true })
  const runs = React.useMemo(() => (runsState.status === "success" ? runsState.data : []), [runsState])
  const summary = React.useMemo(() => summarizeRuns(runs), [runs])
  const anyLive = runs.some(isLiveRun)

  const [now, setNow] = React.useState(() => Date.now())
  const fastUntil = React.useRef(0)
  const reloadRuns = runsState.reload
  React.useEffect(() => {
    const fast = anyLive || Date.now() < fastUntil.current
    const t = setTimeout(
      () => {
        setNow(Date.now())
        reloadRuns()
      },
      fast ? LIVE_POLL_MS : IDLE_POLL_MS
    )
    return () => clearTimeout(t)
  }, [anyLive, reloadRuns, runsState])
  // A live run's clock ticks every second; the rest of the page does not need it.
  React.useEffect(() => {
    if (!anyLive) return
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [anyLive])

  const setParam = React.useCallback(
    (key: string, value: string | null) => {
      const next = new URLSearchParams(search.toString())
      if (value === null) next.delete(key)
      else next.set(key, value)
      const qs = next.toString()
      router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false })
    },
    [pathname, router, search]
  )
  const requestedRun = search.get("run")
  const selectedRun = runs.find((r) => r.id === requestedRun) ?? runs[0] ?? null
  const selectRun = (id: string) => setParam("run", id)

  const runNow = useServiceAction(
    withNotify({ success: "Run launched", error: "The run could not be launched" }, (signal, id: string) =>
      pipelineService.triggerRun(id, signal)
    )
  )
  const pause = useServiceAction(
    withNotify({ success: "Schedule paused", error: "The schedule could not be paused" }, (signal, id: string) =>
      pipelineService.pausePipeline(id, signal)
    )
  )
  const resume = useServiceAction(
    withNotify({ success: "Schedule resumed", error: "The schedule could not be resumed" }, (signal, id: string) =>
      pipelineService.resumePipeline(id, signal)
    )
  )
  const activate = useServiceAction(
    withNotify({ success: "Pipeline marked ready", error: "The pipeline could not be marked ready" }, (signal, id: string) =>
      pipelineService.setPipelineStatus(id, "ready", signal)
    )
  )
  const [pauseOpen, setPauseOpen] = React.useState(false)
  const [deleteOpen, setDeleteOpen] = React.useState(false)
  const remove = useServiceAction(
    withNotify({ success: "Pipeline deleted", error: "The pipeline could not be deleted" }, async (signal, id: string) => {
      await pipelineService.deletePipeline(id, signal)
      return true
    })
  )

  const afterLaunch = React.useCallback(
    (focusNewest: boolean) => {
      fastUntil.current = Date.now() + LAUNCH_WINDOW_MS
      // Follow the new run: dropping `?run=` selects the newest one.
      if (focusNewest) setParam("run", null)
      reloadRuns()
      state.reload()
    },
    [reloadRuns, setParam, state]
  )

  // Step statuses for the Definition tab's graph come from the selected run.
  const stepsState = useService(
    (s) => (selectedRun ? pipelineService.getRunSteps(pipelineId, selectedRun.id, s) : Promise.resolve([])),
    [pipelineId, selectedRun?.id, selectedRun?.status],
    { keepDataOnReload: true }
  )

  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const p = state.data
  const authored = p.engine === "authored"
  const schedule = parseSchedule(p.schedule)
  // A Dagster job's status is its newest run's, never "paused": whether it
  // is paused lives in its schedule's state. An authored pipeline carries
  // "paused" in its own status.
  const schedulePaused = authored
    ? p.status === "paused"
    : schedule.kind === "cron" && schedule.state !== null && !isScheduleRunning(schedule.state)
  const isDraft = p.status === "draft"
  const scheduleText =
    schedule.kind === "manual"
      ? "Manual only"
      : schedule.kind === "cron"
        ? `${localizeScheduleTime(describeCron(schedule.cron) ?? "Cron", p.nextRunAt)} (cron ${schedule.cron})${schedule.state ? `, ${schedule.state.toLowerCase()}` : ""}`
        : schedule.raw

  const tabParam = search.get("tab")
  const tab: TabId =
    tabParam === "definition" || tabParam === "settings" || tabParam === "runs" ? tabParam : authored ? "definition" : "runs"

  // Edit and delete exist only for an authored pipeline: a Dagster job is
  // defined in its code location.
  const authoredMenu = authored ? (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant="outline" size="icon-sm" aria-label="More actions">
            <EllipsisIcon />
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => router.push(`/pipelines/${encodeURIComponent(p.id)}/edit`)}>
          <PencilIcon />
          Edit definition
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onClick={() => setDeleteOpen(true)}>
          <Trash2Icon />
          Delete pipeline
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  ) : null

  const actions = isDraft ? (
    <>
      {authoredMenu}
      <Button
        size="sm"
        disabled={activate.status === "pending"}
        onClick={async () => {
          if (await activate.run(pipelineId)) state.reload()
        }}
      >
        <PlayIcon data-icon="inline-start" />
        {activate.status === "pending" ? "Marking ready…" : "Mark ready"}
      </Button>
    </>
  ) : (
    <>
      {authoredMenu}
      {schedule.kind === "cron" ? (
        schedulePaused ? (
          <Button
            variant="outline"
            size="sm"
            disabled={resume.status === "pending"}
            onClick={async () => {
              if (await resume.run(pipelineId)) state.reload()
            }}
          >
            <PlayIcon data-icon="inline-start" />
            {resume.status === "pending" ? "Resuming…" : "Resume schedule"}
          </Button>
        ) : (
          <Button variant="outline" size="sm" disabled={pause.status === "pending"} onClick={() => setPauseOpen(true)}>
            <PauseIcon data-icon="inline-start" />
            Pause schedule
          </Button>
        )
      ) : null}
      <Button
        size="sm"
        disabled={runNow.status === "pending"}
        onClick={async () => {
          if (await runNow.run(pipelineId)) afterLaunch(true)
        }}
      >
        <PlayIcon data-icon="inline-start" />
        {runNow.status === "pending" ? "Launching…" : "Run now"}
      </Button>
    </>
  )

  return (
    <div className="flex flex-col gap-4">
      <PipelineMasthead
        pipeline={p}
        latest={runs[0] ?? null}
        schedule={schedule}
        schedulePaused={schedulePaused}
        actions={actions}
      />

      {authored && !isDraft && !p.orchestratorJob ? (
        <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
          The orchestrator has not loaded this pipeline&apos;s job yet. Run now asks it to reload first; if it still has
          no job, check that <code className="font-mono">PIPELINE_RUN_TOKEN</code> is set for both the API and the Dagster
          code location.
        </p>
      ) : null}

      {!isDraft ? (
        <PipelineVitals
          summary={summary}
          latest={runs[0] ?? null}
          nextRunAt={p.nextRunAt}
          schedule={schedule}
          schedulePaused={schedulePaused}
          now={now}
        />
      ) : null}

      <Tabs value={tab} onValueChange={(v) => setParam("tab", String(v))}>
        <TabsList variant="line" className="gap-4 border-b border-border pb-1">
          <TabsTrigger value="runs">
            Runs
            {runs.length > 0 ? <span className="font-mono text-[11px] text-muted-foreground">{runs.length}</span> : null}
          </TabsTrigger>
          <TabsTrigger value="definition">Flow &amp; code</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="runs" className="mt-3">
          {runsState.status === "loading" ? (
            <LoadingSkeleton rows={6} />
          ) : runsState.status === "error" ? (
            <ErrorState error={runsState.error} onRetry={runsState.reload} />
          ) : runs.length === 0 ? (
            <EmptyState
              title={isDraft ? "A draft has no runs" : "No runs yet"}
              description={
                isDraft
                  ? "Mark it ready: the orchestrator then builds its job, and it can run on its schedule or on demand."
                  : "The orchestrator has no run of this pipeline. Run it now, or wait for its schedule."
              }
            />
          ) : (
            <div className="flex flex-col gap-4">
              <section className="rounded-xl border border-border bg-card px-4 pb-3 pt-3.5">
                <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
                  <h2 className="font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
                    Run history
                  </h2>
                  <div className="flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                    {(
                      [
                        ["completed", "Completed"],
                        ["failed", "Failed"],
                        ["running", "Running"],
                        ["cancelled", "Cancelled"],
                      ] as [EntityStatus, string][]
                    ).map(([status, label]) => (
                      <span key={status} className="inline-flex items-center gap-1.5">
                        <span className={cn("size-2 rounded-sm", runFill(status))} aria-hidden />
                        {label}
                      </span>
                    ))}
                    <span className="hidden sm:inline">bar height is wall time</span>
                  </div>
                </div>
                <RunHistoryStrip
                  runs={runs}
                  selectedId={selectedRun?.id ?? null}
                  onSelect={(r) => selectRun(r.id)}
                  p50Ms={summary.p50Ms}
                />
              </section>

              {requestedRun && !runs.some((r) => r.id === requestedRun) ? (
                <p className="rounded-lg border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                  Run <span className="font-mono">{requestedRun.slice(0, 8)}</span> is older than the last {runs.length} runs
                  this page reads; showing the newest run instead.
                </p>
              ) : null}

              <div className="grid gap-4 lg:grid-cols-[17rem_minmax(0,1fr)]">
                <aside className="rounded-xl border border-border bg-card p-1.5 lg:sticky lg:top-4 lg:self-start">
                  <RunList runs={runs} selectedId={selectedRun?.id ?? null} onSelect={selectRun} now={now} />
                </aside>
                <div className="min-w-0 rounded-xl border border-border bg-card p-4">
                  {selectedRun ? (
                    <RunInspector
                      key={selectedRun.id}
                      run={selectedRun}
                      now={now}
                      onSelectRun={selectRun}
                      onChanged={afterLaunch}
                    />
                  ) : null}
                </div>
              </div>
            </div>
          )}
        </TabsContent>

        <TabsContent value="definition" className="mt-3">
          {authored ? (
            <AuthoredFlow pipeline={p} />
          ) : (
            <DagsterDefinition
              pipeline={p}
              runLabel={selectedRun ? `run ${selectedRun.id.slice(0, 8)} (${formatRelativeTime(selectedRun.startedAt, now)})` : null}
              steps={selectedRun && stepsState.status === "success" ? stepsState.data : null}
            />
          )}
        </TabsContent>

        <TabsContent value="settings" className="mt-3">
          <SettingsTab p={p} scheduleText={scheduleText} />
          <ScheduleHistory pipelineId={p.id} hasSchedule={schedule.kind === "cron"} onSelectRun={(id) => {
            setParam("tab", "runs")
            selectRun(id)
          }} />
          <p className="mt-3 text-xs text-muted-foreground">
            Lineage for this pipeline:{" "}
            <Link href={`/lineage?focus=${encodeURIComponent(p.id)}`} className="text-primary hover:underline">
              open in Lineage
            </Link>
          </p>
        </TabsContent>
      </Tabs>

      <ConfirmActionDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title="Delete pipeline"
        description={`Delete ${p.name}? Its definition is removed and its job and schedule stop existing.`}
        impact="Past runs stay in the orchestrator's history. This cannot be undone."
        confirmLabel="Delete pipeline"
        confirming={remove.status === "pending"}
        onConfirm={async () => {
          if ((await remove.run(pipelineId)) !== null) {
            setDeleteOpen(false)
            router.push("/pipelines")
          }
        }}
      />
      <ConfirmActionDialog
        open={pauseOpen}
        onOpenChange={setPauseOpen}
        title="Pause schedule"
        description={`Pause ${p.name}'s schedule? It will not fire until resumed.`}
        impact="Runs already in flight continue. Run now still works while paused."
        confirmLabel="Pause schedule"
        confirming={pause.status === "pending"}
        onConfirm={async () => {
          if (await pause.run(pipelineId)) {
            setPauseOpen(false)
            state.reload()
          }
        }}
      />
    </div>
  )
}
