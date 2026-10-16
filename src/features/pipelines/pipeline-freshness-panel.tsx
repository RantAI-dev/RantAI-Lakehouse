"use client"

import * as React from "react"
import { MetadataList } from "@/components/patterns/metadata-list"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { pipelineService } from "@/services"
import type { PipelineDetail, TargetLag } from "@/services/contracts/pipelines"

/**
 * The bounds the console offers, and the API accepts.
 *
 * BLD-2 review round 1 / plan D11: the floor is 2 minutes because the
 * acceptance gate measured a worst sample of 68.94 s on the compose stack
 * and a 1-minute target is not met there. They live here, next to the input
 * that uses them, so the panel, its validation message and the API's own
 * bound (`MIN_TARGET_LAG_MINUTES`) name the same number.
 */
const MIN_MINUTES = 2
const MAX_MINUTES = 10080

/**
 * The minutes a `lag` target lag carries, or `null` when it carries none.
 *
 * `TargetLag` is widened with `| string` (D2, review R1 BLOCKER 10), so a
 * stored value this contract does not know about has no `minutes` to read.
 * Reading through this one helper keeps the panel honest — it falls back to
 * the default it would offer anyway instead of rendering "undefined
 * minutes".
 */
function lagMinutes(targetLag: TargetLag | null | undefined): number | null {
  if (!targetLag || targetLag.kind !== "lag") return null
  return typeof targetLag.minutes === "number" ? targetLag.minutes : null
}

export function PipelineFreshnessPanel({
  pipeline,
  onUpdated,
}: {
  pipeline: PipelineDetail
  onUpdated?: () => void
}) {
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("pipeline:write")
  const isAuthored = pipeline.engine === "authored"

  const [isEditing, setIsEditing] = React.useState(false)
  const [mode, setMode] = React.useState<"none" | "lag" | "downstream">(() => {
    if (!pipeline.targetLag) return "none"
    return pipeline.targetLag.kind === "lag" ? "lag" : "downstream"
  })
  const [minutes, setMinutes] = React.useState<number>(
    () => lagMinutes(pipeline.targetLag) ?? 10
  )
  const [localError, setLocalError] = React.useState<string | null>(null)

  // BLD-2 review R1 BLOCKER 10: use useServiceAction
  const saveAction = useServiceAction(async (signal, payload: TargetLag | null) => {
    return pipelineService.setTargetLag(pipeline.id, payload, signal)
  })

  const saving = saveAction.status === "pending"
  const error = localError || saveAction.error?.message || null
  // `reset` is a stable `useCallback`; depending on it in the sync effect
  // below is what keeps the React compiler from assuming the effect is
  // missing a dependency.
  const { reset: resetSave } = saveAction

  // Sync state if pipeline changes externally
  React.useEffect(() => {
    if (!isEditing) {
      const configuredMinutes = lagMinutes(pipeline.targetLag)
      if (!pipeline.targetLag) {
        setMode("none")
        setMinutes(10)
      } else if (configuredMinutes != null) {
        setMode("lag")
        setMinutes(configuredMinutes)
      } else {
        setMode("downstream")
        setMinutes(10)
      }
      setLocalError(null)
      resetSave()
    }
  }, [pipeline.targetLag, isEditing, resetSave])

  const targetLagDesc = React.useMemo(() => {
    if (!pipeline.targetLag) return "None (runs on schedule or demand)"
    const configuredMinutes = lagMinutes(pipeline.targetLag)
    if (configuredMinutes != null) {
      return `${configuredMinutes} minute${configuredMinutes === 1 ? "" : "s"}`
    }
    if (pipeline.targetLag.kind === "downstream") return "Downstream"
    // A `kind` this build does not know is shown as such rather than
    // silently read as `downstream` (AGENTS.md principle 2).
    return "Unknown target lag"
  }, [pipeline.targetLag])

  // No `React.useMemo` here: the React compiler memoizes these itself and
  // rejects a hand-written dependency list that does not match what the
  // body reads (`react-hooks/preserve-manual-memoization`).
  const currentLagDesc =
    pipeline.lag?.lagSeconds != null ? `${pipeline.lag.lagSeconds}s` : "—"

  const lastCheckDesc = pipeline.lag?.lastCheckAt
    ? formatDateTime(pipeline.lag.lastCheckAt)
    : "—"

  const statusPill = React.useMemo(() => {
    if (!pipeline.targetLag) {
      return <Pill tone="neutral">Not configured</Pill>
    }
    const lag = pipeline.lag
    if (!lag) return <Pill tone="neutral">Not measured</Pill>
    if (lag.status === "met") return <Pill tone="success" title={lag.reason ?? undefined}>Met</Pill>
    if (lag.status === "behind") return <Pill tone="destructive" title={lag.reason ?? undefined}>Behind</Pill>
    return <Pill tone="neutral" title={lag.reason ?? undefined}>Not measured</Pill>
  }, [pipeline.targetLag, pipeline.lag])

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault()
    setLocalError(null)

    let payload: TargetLag | null = null
    if (mode === "lag") {
      const parsedMinutes = Number(minutes)
      // BLD-2 review round 1 / D11: the floor is the measured one (the gate
      // did not meet 1 minute on this stack), not the spec's 1.
      if (isNaN(parsedMinutes) || parsedMinutes < MIN_MINUTES || parsedMinutes > MAX_MINUTES) {
        setLocalError(`Minutes must be between ${MIN_MINUTES} and ${MAX_MINUTES}`)
        return
      }
      payload = { kind: "lag", minutes: parsedMinutes }
    } else if (mode === "downstream") {
      payload = { kind: "downstream" }
    }

    const res = await saveAction.run(payload)
    if (res) {
      setIsEditing(false)
      onUpdated?.()
    }
  }

  const handleCancel = () => {
    setIsEditing(false)
    setLocalError(null)
    resetSave()
    const configuredMinutes = lagMinutes(pipeline.targetLag)
    if (!pipeline.targetLag) {
      setMode("none")
      setMinutes(10)
    } else if (configuredMinutes != null) {
      setMode("lag")
      setMinutes(configuredMinutes)
    } else {
      setMode("downstream")
      setMinutes(10)
    }
  }

  return (
    <section className="rounded-xl border border-border bg-card p-4" aria-label="Freshness">
      <div className="flex items-center justify-between gap-4 mb-3">
        <div className="flex items-center gap-2">
          <h3 className="font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
            Freshness
          </h3>
          {statusPill}
        </div>
        {canWrite && isAuthored && !isEditing ? (
          <Button
            variant="outline"
            size="sm"
            onClick={() => setIsEditing(true)}
            aria-label="Configure target lag"
          >
            Configure target lag
          </Button>
        ) : null}
      </div>

      {isEditing ? (
        <form onSubmit={handleSave} className="flex flex-col gap-4 pt-2">
          <div className="flex flex-col gap-2">
            <Label className="text-xs font-semibold text-foreground">Target lag type</Label>
            <div className="flex flex-wrap gap-2" role="radiogroup" aria-label="Target lag mode">
              <button
                type="button"
                role="radio"
                aria-checked={mode === "none"}
                onClick={() => setMode("none")}
                className={`rounded-lg border px-3 py-1.5 text-xs transition-colors ${
                  mode === "none"
                    ? "border-primary bg-primary/10 text-foreground font-medium"
                    : "border-border text-muted-foreground hover:border-primary/40"
                }`}
              >
                None (clear)
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={mode === "lag"}
                onClick={() => setMode("lag")}
                className={`rounded-lg border px-3 py-1.5 text-xs transition-colors ${
                  mode === "lag"
                    ? "border-primary bg-primary/10 text-foreground font-medium"
                    : "border-border text-muted-foreground hover:border-primary/40"
                }`}
              >
                Target lag (minutes)
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={mode === "downstream"}
                onClick={() => setMode("downstream")}
                className={`rounded-lg border px-3 py-1.5 text-xs transition-colors ${
                  mode === "downstream"
                    ? "border-primary bg-primary/10 text-foreground font-medium"
                    : "border-border text-muted-foreground hover:border-primary/40"
                }`}
              >
                Downstream
              </button>
            </div>
          </div>

          {mode === "lag" ? (
            <div className="flex flex-col gap-1.5 max-w-xs">
              <Label htmlFor="target-lag-minutes" className="text-xs">
                Minutes ({MIN_MINUTES} - {MAX_MINUTES})
              </Label>
              <Input
                id="target-lag-minutes"
                type="number"
                min={MIN_MINUTES}
                max={MAX_MINUTES}
                value={minutes}
                onChange={(e) => setMinutes(Number(e.target.value))}
                className="h-8 text-xs font-mono"
                required
              />
            </div>
          ) : null}

          {mode === "downstream" ? (
            <p className="text-xs text-muted-foreground">
              Refreshes only when a downstream pipeline with a target lag reads this table.
            </p>
          ) : null}

          {error ? (
            <div
              role="alert"
              className="rounded-md border border-destructive/30 bg-destructive/10 p-2.5 text-xs text-destructive font-medium"
            >
              {error}
            </div>
          ) : null}

          <div className="flex items-center gap-2 pt-1">
            <Button type="submit" size="sm" disabled={saving}>
              {saving ? "Saving…" : "Save target lag"}
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={handleCancel} disabled={saving}>
              Cancel
            </Button>
          </div>
        </form>
      ) : (
        <div className="flex flex-col gap-3">
          <MetadataList
            columns={3}
            items={[
              { label: "Target lag", value: targetLagDesc },
              { label: "Current lag", value: currentLagDesc },
              { label: "Last check", value: lastCheckDesc },
            ]}
          />
          {pipeline.lag?.reason ? (
            <div className="rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground" role="note">
              {pipeline.lag.reason}
            </div>
          ) : null}
        </div>
      )}
    </section>
  )
}
