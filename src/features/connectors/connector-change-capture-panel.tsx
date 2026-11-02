"use client"

import * as React from "react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useService, useServiceAction } from "@/hooks/use-service"
import { isValidLagLimit, lagTone, lagWords } from "@/lib/connectors/capture-lag"
import { notifySuccess } from "@/lib/notify"
import { connectorService } from "@/services"
import type { CaptureTableLag, ChangeCaptureLag } from "@/services/contracts/connectors"
import { ConnectorSetupChecks } from "./connector-setup-checks"
import { isLiftablePause } from "./connector-ingest-panel"

/** How often the tab asks again: the orchestrator reports every 30 seconds, so this is about every second report. */
const REFRESH_MS = 60_000

/** The words for the states `GET /api/connectors/{id}/change-capture` answers. Anything else is shown as sent. */
const CAPTURE_STATE_WORDS: Record<string, { label: string; tone: "success" | "warning" | "destructive" | "neutral"; note: string }> = {
  running: { label: "Running", tone: "success", note: "Changes at the source are being captured." },
  paused: { label: "Paused", tone: "warning", note: "Capture is stopped; the changes made meanwhile are kept and applied after Resume." },
  failed: { label: "Failed", tone: "destructive", note: "Capture stopped on an error. The source's recent changes are kept for seven days." },
  starting: { label: "Starting", tone: "neutral", note: "Capture is starting. The first copy of each table is taken now." },
  not_created: { label: "Not started", tone: "neutral", note: "Capture starts when the tables are saved." },
  unreachable: { label: "Not reachable", tone: "warning", note: "The capture service did not answer; this is not a statement about the connector." },
}

/**
 * The connector's change capture, on a tab of its own (`SRC-4` decision S4-g): its state in the capture service in
 * fixed words with Pause and Resume (moved here from the Ingest tab), how far behind the source each table is, and
 * the lag at which an alert is sent. A deployment without the capture service says so (`supported: false`), and
 * nothing is shown as running or up to date that the API did not say.
 */
export function ConnectorChangeCapturePanel({
  connectorId,
  pausedReason,
  onChanged,
}: {
  connectorId: string
  pausedReason: string | null
  onChanged?: () => void
}) {
  const capture = useService(
    (signal) => connectorService.getChangeCapture(connectorId, signal),
    [connectorId, pausedReason],
    { keepDataOnReload: true }
  )
  const { reload } = capture
  React.useEffect(() => {
    const timer = setInterval(reload, REFRESH_MS)
    return () => clearInterval(timer)
  }, [reload])

  // Slice 4 review SHOULD-FIX 5: Pause and Resume need no state read (the routes do not), so a
  // paused connector keeps its Resume when the read fails or says unsupported.
  const controls = (
    <CaptureState connectorId={connectorId} state={undefined} pausedReason={pausedReason} onChanged={() => onChanged?.()} />
  )
  if (capture.status === "loading") return <LoadingSkeleton rows={3} />
  if (capture.status === "error") {
    return (
      <div className="flex flex-col gap-4">
        <ErrorState error={capture.error} onRetry={capture.reload} />
        {controls}
      </div>
    )
  }
  const data = capture.data
  if (!data.supported) {
    return (
      <div className="flex flex-col gap-4">
        <p className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          {data.reason ?? "Change capture is not available for this connector."}
        </p>
        {controls}
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-4">
      <CaptureState
        connectorId={connectorId}
        state={data.state}
        pausedReason={pausedReason}
        onChanged={() => {
          onChanged?.()
          capture.reload()
        }}
      />
      {data.setupChecks && data.setupChecks.length > 0 ? (
        <ConnectorSetupChecks checks={data.setupChecks} />
      ) : null}
      {data.lag ? (
        <>
          <LagTable lag={data.lag} />
          <LagLimit connectorId={connectorId} lag={data.lag} onSaved={capture.reload} />
        </>
      ) : null}
    </div>
  )
}

function CaptureState({
  connectorId,
  state,
  pausedReason,
  onChanged,
}: {
  connectorId: string
  state: string | undefined
  pausedReason: string | null
  onChanged: () => void
}) {
  const pause = useServiceAction(async (signal) => {
    const result = await connectorService.pauseConnector(connectorId, signal)
    if (!signal.aborted) notifySuccess("Paused; the changes made meanwhile are kept")
    return result
  })
  const resume = useServiceAction(async (signal) => {
    const result = await connectorService.resumeConnector(connectorId, signal)
    if (!signal.aborted) notifySuccess("Resumed; the changes kept meanwhile are applied")
    return result
  })
  const word = CAPTURE_STATE_WORDS[state ?? ""]
  const failure = pause.status === "error" ? pause.error : resume.status === "error" ? resume.error : null
  return (
    <SectionCard title="Capture">
      <div className="space-y-1.5">
        <div className="flex flex-wrap items-center gap-2">
          <Pill tone={word?.tone ?? "neutral"}>{word?.label ?? state ?? "Unknown"}</Pill>
          {!pausedReason ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={pause.status === "pending"}
              onClick={async () => {
                if (await pause.run()) onChanged()
              }}
            >
              {pause.status === "pending" ? "Pausing…" : "Pause"}
            </Button>
          ) : null}
          {pausedReason && isLiftablePause(pausedReason) ? (
            <Button
              type="button"
              size="sm"
              disabled={resume.status === "pending"}
              onClick={async () => {
                if (await resume.run()) onChanged()
              }}
            >
              {resume.status === "pending" ? "Resuming…" : "Resume"}
            </Button>
          ) : null}
        </div>
        <p className="text-xs text-muted-foreground">{word?.note ?? ""}</p>
        {pausedReason ? <p className="text-xs">Paused: {pausedReason}</p> : null}
        {failure ? (
          <p role="alert" className="text-xs text-destructive">
            {failure.message}
          </p>
        ) : null}
      </div>
    </SectionCard>
  )
}

function LagTable({ lag }: { lag: ChangeCaptureLag }) {
  return (
    <SectionCard
      title="Lag"
      description="How far each table is behind the source. A table is up to date when nothing waits to be applied and capture is running."
    >
      {lag.tables.length === 0 ? (
        <p className="text-xs text-muted-foreground">No table is selected yet.</p>
      ) : (
        <ul className="divide-y divide-border">
          {lag.tables.map((table: CaptureTableLag) => (
            <li key={table.table} className="flex flex-wrap items-center justify-between gap-2 py-1.5">
              <span className="min-w-0 break-all text-sm">{table.table}</span>
              <Pill tone={lagTone(table, lag.limitMinutes, table.seconds)}>{lagWords(table)}</Pill>
            </li>
          ))}
        </ul>
      )}
    </SectionCard>
  )
}

function LagLimit({ connectorId, lag, onSaved }: { connectorId: string; lag: ChangeCaptureLag; onSaved: () => void }) {
  const [value, setValue] = React.useState(String(lag.limitMinutes))
  const save = useServiceAction(async (signal, minutes: number) => {
    const result = await connectorService.setLagLimit(connectorId, minutes, signal)
    if (!signal.aborted) notifySuccess("Lag limit saved")
    return result
  })
  const valid = isValidLagLimit(value)
  const unchanged = valid && Number(value) === lag.limitMinutes
  return (
    <SectionCard
      title="Lag alert"
      description="An alert is sent once when a table is further behind than this, and once more when it is back under."
    >
      <form
        className="flex flex-wrap items-end gap-2"
        onSubmit={async (event) => {
          event.preventDefault()
          if (valid && (await save.run(Number(value)))) onSaved()
        }}
      >
        <label className="space-y-1 text-xs text-muted-foreground">
          Alert after (minutes)
          <Input
            inputMode="numeric"
            className="w-28"
            value={value}
            aria-invalid={!valid}
            onChange={(event) => setValue(event.target.value)}
          />
        </label>
        <Button type="submit" size="sm" disabled={!valid || unchanged || save.status === "pending"}>
          {save.status === "pending" ? "Saving…" : "Save"}
        </Button>
      </form>
      {!valid ? <p className="mt-1 text-xs text-destructive">Use a whole number of minutes from 1 to 1440.</p> : null}
      {lag.alertOpen ? (
        <p className="mt-1 text-xs text-amber-600 dark:text-amber-400">
          An alert is open: a table passed this limit and has not come back under it.
        </p>
      ) : null}
      {save.status === "error" ? (
        <p role="alert" className="mt-1 text-xs text-destructive">
          {save.error.message}
        </p>
      ) : null}
    </SectionCard>
  )
}
