"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { listTimeZones } from "@/lib/time-zones"
import { withNotify } from "@/lib/notify"
import { settingsService } from "@/services"

const WEEK_STARTS = { monday: "Monday", sunday: "Sunday" } as const

/**
 * Admin > Settings (`BI-9`): the report time zone and the first day of the
 * week for the whole deployment. Anyone signed in sees the values; only a
 * holder of `settings:write` can change them (the server enforces it; the
 * controls are disabled here so the page does not offer what would be
 * refused).
 */
export function ReportingSettingsPage() {
  const { hasPermission } = useAuth()
  return <ReportingSettingsForm canWrite={hasPermission("settings:write")} />
}

export function ReportingSettingsForm({
  canWrite,
  zoneList = listTimeZones,
}: {
  canWrite: boolean
  /**
   * BI-9 review fix R6: where the zone names come from. Tests pass a short
   * list, because several hundred `<option>`s are slow to render under load;
   * the page always uses the browser's full list.
   */
  zoneList?: (saved?: string) => string[]
}) {
  const state = useService((signal) => settingsService.getReporting(signal), [])
  const [zone, setZone] = React.useState("")
  const [week, setWeek] = React.useState("monday")
  const save = useServiceAction(
    withNotify(
      { success: "Settings saved", error: "Settings were not saved" },
      (signal, input: { timeZone: string; weekStart: string }) =>
        settingsService.saveReporting(input, signal)
    )
  )

  const loaded = state.status === "success" ? state.data : null
  React.useEffect(() => {
    if (loaded) {
      setZone(loaded.timeZone)
      setWeek(loaded.weekStart)
    }
  }, [loaded])

  const zones = React.useMemo(() => zoneList(loaded?.timeZone), [zoneList, loaded?.timeZone])
  const changed = loaded !== null && (zone !== loaded.timeZone || week !== loaded.weekStart)

  async function handleSave() {
    const result = await save.run({ timeZone: zone, weekStart: week })
    if (result) state.reload()
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Settings"
        description="Used by every dashboard, for grouping dates and for “today”."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {loaded ? (
        <div className="flex max-w-sm flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="report-time-zone">Report time zone</Label>
            {/* A native datalist: typing filters the 400-odd names, with no
                popup to manage. The server still checks the name. */}
            <Input
              id="report-time-zone"
              aria-label="Report time zone"
              list="report-time-zones"
              value={zone}
              onChange={(e) => setZone(e.target.value)}
              placeholder="Search a time zone"
              disabled={!canWrite}
              autoComplete="off"
            />
            <datalist id="report-time-zones">
              {zones.map((name) => (
                <option key={name} value={name} />
              ))}
            </datalist>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>First day of the week</Label>
            <Select
              value={week}
              items={WEEK_STARTS}
              onValueChange={(v) => setWeek(v ?? week)}
              disabled={!canWrite}
            >
              <SelectTrigger className="w-full" aria-label="First day of the week">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.entries(WEEK_STARTS).map(([value, label]) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {canWrite ? (
            <Button
              className="w-fit"
              onClick={handleSave}
              disabled={!changed || save.status === "pending"}
            >
              Save
            </Button>
          ) : (
            <p className="text-sm text-muted-foreground">
              Only an administrator can change these.
            </p>
          )}
        </div>
      ) : null}
    </div>
  )
}
