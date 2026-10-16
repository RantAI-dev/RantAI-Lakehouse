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
  const [minutes, setMinutes] = React.useState<number>(() => {
    if (pipeline.targetLag?.kind === "lag") return pipeline.targetLag.minutes
    return 10
  })
  const [localError, setLocalError] = React.useState<string | null>(null)

  // BLD-2 review R1 BLOCKER 10: use useServiceAction
  const saveAction = useServiceAction(async (signal, payload: TargetLag | null) => {
    return pipelineService.setTargetLag(pipeline.id, payload, signal)
  })

  const saving = saveAction.status === "pending"
  const error = localError || saveAction.error?.message || null

  // Sync state if pipeline changes externally
  React.useEffect(() => {
    if (!isEditing) {
      if (!pipeline.targetLag) {
        setMode("none")
        setMinutes(10)
      } else if (pipeline.targetLag.kind === "lag") {
        setMode("lag")
        setMinutes(pipeline.targetLag.minutes)
      } else {
        setMode("downstream")
        setMinutes(10)
      }
      setLocalError(null)
      saveAction.reset()
    }
  }, [pipeline.targetLag, isEditing])

  const targetLagDesc = React.useMemo(() => {
    if (!pipeline.targetLag) return "None (runs on schedule or demand)"
    if (pipeline.targetLag.kind === "downstream") return "Downstream"
    return `${pipeline.targetLag.minutes} minute${pipeline.targetLag.minutes === 1 ? "" : "s"}`
  }, [pipeline.targetLag])

  const currentLagDesc = React.useMemo(() => {
    if (pipeline.lag?.lagSeconds != null) {
      return `${pipeline.lag.lagSeconds}s`
    }
    return "—"
  }, [pipeline.lag?.lagSeconds])

  const lastCheckDesc = React.useMemo(() => {
    if (pipeline.lag?.lastCheckAt) {
      return formatDateTime(pipeline.lag.lastCheckAt)
    }
    return "—"
  }, [pipeline.lag?.lastCheckAt])

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
      if (isNaN(parsedMinutes) || parsedMinutes < 1 || parsedMinutes > 10080) {
        setLocalError("Minutes must be between 1 and 10080")
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
    saveAction.reset()
    if (!pipeline.targetLag) {
      setMode("none")
      setMinutes(10)
    } else if (pipeline.targetLag.kind === "lag") {
      setMode("lag")
      setMinutes(pipeline.targetLag.minutes)
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
              <Label htmlFor="target-lag-minutes" className="text-xs">Minutes (1 - 10080)</Label>
              <Input
                id="target-lag-minutes"
                type="number"
                min={1}
                max={10080}
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
