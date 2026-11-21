"use client"

import * as React from "react"
import { SelectField } from "@/components/patterns/select-field"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { governanceService } from "@/services"
import type { QualitySchedule } from "@/services/contracts/governance"

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
 * The control that changes one table's schedule: on a timer or on every
 * load, and how often. Shared by the asset page's panel below and Data
 * Quality's Schedules tab (`WEB-3` S3-D5), so both save the same way.
 * It starts on what is saved; `onSaved` runs after the server took it.
 */
export function ScheduleControls({
  asset,
  current,
  defaultInterval,
  onSaved,
}: {
  asset: string
  current: QualitySchedule | undefined
  defaultInterval: number
  onSaved: () => void
}) {
  const id = React.useId()
  const [mode, setMode] = React.useState<"interval" | "on_load">(
    current?.mode === "on_load" ? "on_load" : "interval"
  )
  const [minutes, setMinutes] = React.useState<number>(current?.intervalMinutes ?? defaultInterval)
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
    onSaved()
  }

  return (
    <div className="flex flex-wrap items-end gap-3">
      <div className="grid gap-1">
        <Label htmlFor={`${id}-mode`}>Run the checks</Label>
        <SelectField
          id={`${id}-mode`}
          className="w-full"
          value={mode}
          options={[
            { value: "interval", label: "On a timer" },
            { value: "on_load", label: "On every load" },
          ]}
          onValueChange={(next) => setMode(next as "interval" | "on_load")}
        />
      </div>
      <div className="grid gap-1">
        <Label htmlFor={`${id}-interval`}>{mode === "on_load" ? "And at least every" : "Every"}</Label>
        <SelectField
          id={`${id}-interval`}
          className="w-full"
          value={String(minutes)}
          options={choices.map((m) => ({ value: String(m), label: intervalLabel(m) }))}
          onValueChange={(next) => setMinutes(Number(next))}
        />
      </div>
      <Button size="sm" variant="outline" onClick={() => void submit()} disabled={save.status === "pending"}>
        {save.status === "pending" ? "Saving…" : "Save schedule"}
      </Button>
      {save.error ? <p className="w-full text-destructive">{save.error.message}</p> : null}
    </div>
  )
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
        // Mounted once the schedule has loaded, so it starts on what is saved.
        <ScheduleControls
          asset={asset}
          current={current}
          defaultInterval={defaultInterval}
          onSaved={schedules.reload}
        />
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
