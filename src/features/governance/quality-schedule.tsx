"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { governanceService } from "@/services"
import type { QualitySchedule } from "@/services/contracts/governance"

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

/** The intervals offered, in minutes: one hour to one day. */
export const INTERVAL_CHOICES = [60, 120, 180, 360, 720, 1440] as const

/** `60` is "1 hour", `1440` "1 day", `90` "90 minutes". */
export function intervalLabel(minutes: number): string {
  if (minutes % 1440 === 0) {
    const days = minutes / 1440
    return days === 1 ? "1 day" : `${days} days`
  }
  if (minutes % 60 === 0) {
    const hours = minutes / 60
    return hours === 1 ? "1 hour" : `${hours} hours`
  }
  return `${minutes} minutes`
}

/** What the table's schedule is, in a sentence. */
export function scheduleSentence(
  schedule: QualitySchedule | undefined,
  defaultIntervalMinutes: number
): string {
  if (!schedule) return `Every ${intervalLabel(defaultIntervalMinutes)} (the default).`
  if (schedule.mode === "on_load") {
    return `After every load of the table, and every ${intervalLabel(schedule.intervalMinutes)} as a backstop.`
  }
  return `Every ${intervalLabel(schedule.intervalMinutes)}.`
}

/**
 * When a table's checks run, and whether its volume and freshness anomalies
 * are being raised yet (`DATA-14`, F3, F5 and F7). Anyone can read both; the
 * control to change the schedule needs `governance:write`.
 */
export function QualitySchedulePanel({
  asset,
  canWrite,
}: {
  asset: string
  canWrite: boolean
}) {
  const schedules = useService((signal) => governanceService.listQualitySchedules(signal), [asset])
  const learning = useService((signal) => governanceService.getQualityLearning(asset, signal), [asset])
  const key = asset.toLowerCase()
  const current =
    schedules.status === "success"
      ? schedules.data.schedules.find((s) => s.asset === key)
      : undefined
  const defaultInterval = schedules.status === "success" ? schedules.data.defaultIntervalMinutes : 60

  const [mode, setMode] = React.useState<"interval" | "on_load">("interval")
  const [minutes, setMinutes] = React.useState<number>(defaultInterval)
  const [seeded, setSeeded] = React.useState(false)
  // Start the control on what is saved, once it has loaded.
  React.useEffect(() => {
    if (schedules.status !== "success" || seeded) return
    setMode(current?.mode === "on_load" ? "on_load" : "interval")
    setMinutes(current?.intervalMinutes ?? defaultInterval)
    setSeeded(true)
  }, [schedules.status, seeded, current, defaultInterval])

  const save = useServiceAction((signal, input: Parameters<typeof governanceService.setQualitySchedule>[1]) =>
    governanceService.setQualitySchedule(asset, input, signal)
  )
  const choices = [...new Set<number>([...INTERVAL_CHOICES, minutes])].sort((a, b) => a - b)

  async function submit() {
    const saved = await save.run(
      mode === "on_load"
        ? { mode: "on_load", intervalMinutes: minutes }
        : { mode: "interval", intervalMinutes: minutes }
    )
    if (saved === null) return
    notifySuccess("Schedule saved")
    schedules.reload()
  }

  return (
    <div className="mb-3 grid gap-2 rounded-lg border border-border px-3 py-2 text-sm">
      <div>
        <span className="font-medium">Checks run: </span>
        {schedules.status === "success" ? (
          scheduleSentence(current, defaultInterval)
        ) : schedules.status === "error" ? (
          <span className="text-destructive">
            the schedule could not be loaded: {schedules.error.message}
          </span>
        ) : (
          <span className="text-muted-foreground">loading…</span>
        )}
      </div>
      {canWrite && schedules.status === "success" ? (
        <div className="flex flex-wrap items-end gap-3">
          <div className="grid gap-1">
            <Label htmlFor="schedule-mode">Run the checks</Label>
            <select
              id="schedule-mode"
              className={selectClassName}
              value={mode}
              onChange={(e) => setMode(e.target.value as "interval" | "on_load")}
            >
              <option value="interval">On a timer</option>
              <option value="on_load">On every load</option>
            </select>
          </div>
          <div className="grid gap-1">
            <Label htmlFor="schedule-interval">
              {mode === "on_load" ? "And at least every" : "Every"}
            </Label>
            <select
              id="schedule-interval"
              className={selectClassName}
              value={minutes}
              onChange={(e) => setMinutes(Number(e.target.value))}
            >
              {choices.map((m) => (
                <option key={m} value={m}>
                  {intervalLabel(m)}
                </option>
              ))}
            </select>
          </div>
          <Button size="sm" variant="outline" onClick={() => void submit()} disabled={save.status === "pending"}>
            {save.status === "pending" ? "Saving…" : "Save schedule"}
          </Button>
          {save.error ? <p className="w-full text-destructive">{save.error.message}</p> : null}
        </div>
      ) : null}
      <div>
        <span className="font-medium">Anomaly detection: </span>
        {learning.status === "success" ? (
          learning.data.learned ? (
            <>
              on. Learned from {learning.data.days} days and {learning.data.loads} loads; a load far
              smaller than usual, or a long silence, opens an incident.
            </>
          ) : (
            (learning.data.message ?? "learning")
          )
        ) : learning.status === "error" ? (
          <span className="text-destructive">
            the state could not be loaded: {learning.error.message}
          </span>
        ) : (
          <span className="text-muted-foreground">loading…</span>
        )}
      </div>
    </div>
  )
}
