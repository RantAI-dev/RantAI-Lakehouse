"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { describeCron, parseSchedule } from "./pipeline-schedule"

/**
 * The presets an authored pipeline's schedule is picked from. Each is the
 * exact value stored and handed to the orchestrator: `authored_factory.py`
 * builds a schedule only for a five-field cron, so the old free-text field
 * (default "Every hour") saved labels that read like a schedule and never
 * fired.
 */
const PRESETS: { value: string; label: string }[] = [
  { value: "manual", label: "On demand only" },
  { value: "*/15 * * * *", label: "Every 15 minutes" },
  { value: "0 * * * *", label: "Hourly" },
  { value: "0 2 * * *", label: "Daily at 02:00" },
  { value: "0 2 * * 1", label: "Weekly, Monday 02:00" },
]

const CUSTOM = "custom"

/** Whether `value` is something the orchestrator can schedule, or "manual". */
export function isSchedulable(value: string): boolean {
  const parsed = parseSchedule(value)
  return parsed.kind === "manual" || parsed.kind === "cron"
}

/**
 * A schedule picker: presets, or a custom five-field cron read back in
 * words when it is a pattern `describeCron` knows. Times are UTC: the
 * factory sets no `execution_timezone`, and Dagster's default is UTC.
 */
export function PipelineScheduleField({
  value,
  onChange,
}: {
  value: string
  onChange: (value: string) => void
}) {
  const preset = PRESETS.find((p) => p.value === value.trim())
  const [mode, setMode] = React.useState<string>(preset ? preset.value : CUSTOM)
  const parsed = parseSchedule(value)
  const words = parsed.kind === "cron" ? describeCron(parsed.cron) : null
  const invalid = mode === CUSTOM && parsed.kind === "other"

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-2" role="radiogroup" aria-label="Schedule">
        {[...PRESETS, { value: CUSTOM, label: "Custom cron" }].map((p) => (
          <button
            key={p.value}
            type="button"
            role="radio"
            aria-checked={mode === p.value}
            onClick={() => {
              setMode(p.value)
              if (p.value !== CUSTOM) onChange(p.value)
              else if (preset) onChange(preset.value === "manual" ? "0 * * * *" : preset.value)
            }}
            className={cn(
              "rounded-lg border px-3 py-1.5 text-sm transition-colors",
              mode === p.value
                ? "border-primary bg-primary/10 text-foreground"
                : "border-border text-muted-foreground hover:border-primary/40 hover:text-foreground"
            )}
          >
            {p.label}
          </button>
        ))}
      </div>
      {mode === CUSTOM ? (
        <div className="flex flex-col gap-1.5">
          <Input
            value={value}
            onChange={(e) => onChange(e.target.value)}
            placeholder="30 1 * * *"
            aria-invalid={invalid}
            className="font-mono"
          />
          <p className={cn("text-xs", invalid ? "text-destructive" : "text-muted-foreground")}>
            {invalid
              ? "Five fields: minute hour day-of-month month day-of-week."
              : words
                ? `${words} (UTC).`
                : "A valid cron, in UTC. It has no plain-words reading here."}
          </p>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          {parsed.kind === "manual"
            ? "Runs only when someone clicks Run now or the Copilot launches it."
            : `Cron ${value}; times are UTC, the orchestrator's schedule timezone.`}
        </p>
      )}
    </div>
  )
}
