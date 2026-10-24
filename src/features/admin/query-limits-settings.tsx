"use client"

import * as React from "react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { LIMIT_RANGES, limitProblem, type LimitField } from "@/lib/query-limits"
import { withNotify } from "@/lib/notify"
import { identityService, settingsService } from "@/services"
import type { QueryLimits, QueryLimitsInput } from "@/services/contracts/settings"

const FIELDS = Object.keys(LIMIT_RANGES) as LimitField[]

type Draft = Record<LimitField, string>
type RoleRow = { roleId: string; timeoutSeconds: string }

const draftOf = (l: QueryLimits & { install: NonNullable<QueryLimits["install"]> }): Draft => ({
  timeoutSeconds: String(l.install.timeoutSeconds),
  chartRows: String(l.install.chartRows),
  mapPoints: String(l.install.mapPoints),
  studioRows: String(l.install.studioRows),
})
const rowsOf = (l: QueryLimits): RoleRow[] => (l.roles ?? []).map((r) => ({ roleId: r.roleId, timeoutSeconds: String(r.timeoutSeconds) }))

/**
 * Admin > Settings > Query limits (`BI-5`, feature decisions 1, 2 and 6): how
 * long a chart or a Query Studio query may run and how many rows or points it
 * reads, for the whole install, and a longer or shorter time for chosen
 * roles. Anyone signed in sees the limits that apply to them; only a holder of
 * `settings:write` can change them (the server enforces it; the controls are
 * disabled here so the page does not offer what would be refused). The ranges
 * are checked here to say the range before a save, and again by the server.
 *
 * The route (`/api/settings/limits`, T1 of the BI-5 plan) is not on the server
 * yet; until it is, this section shows the read error and nothing else.
 */
export function QueryLimitsSettings() {
  const { hasPermission } = useAuth()
  return <QueryLimitsForm canWrite={hasPermission("settings:write")} />
}

export function QueryLimitsForm({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => settingsService.getLimits(signal), [])
  const roles = useService((signal) => identityService.listRoles(signal), [])
  const [draft, setDraft] = React.useState<Draft | null>(null)
  const [rows, setRows] = React.useState<RoleRow[]>([])
  const [pick, setPick] = React.useState("")
  const save = useServiceAction(
    withNotify({ success: "Query limits saved", error: "Query limits were not saved" }, (signal, input: QueryLimitsInput) =>
      settingsService.saveLimits(input, signal)
    )
  )

  const loaded = state.status === "success" ? state.data : null
  React.useEffect(() => {
    // A reader without settings:write gets only what applies to them (no install row, no role rows).
    if (loaded?.install) {
      setDraft(draftOf({ ...loaded, install: loaded.install }))
      setRows(rowsOf(loaded))
    }
  }, [loaded])

  const roleList = roles.status === "success" && Array.isArray(roles.data) ? roles.data : []
  const nameOf = (id: string) => roleList.find((r) => r.id === id)?.name ?? id
  const free = roleList.filter((r) => !rows.some((row) => row.roleId === r.id))

  const problems = draft
    ? [
        ...FIELDS.map((f) => limitProblem(f, draft[f])),
        ...rows.map((r) => limitProblem("timeoutSeconds", r.timeoutSeconds)),
      ].filter((p): p is string => p !== null)
    : []
  const changed =
    loaded?.install !== undefined &&
    draft !== null &&
    (JSON.stringify(draft) !== JSON.stringify(draftOf({ ...loaded, install: loaded.install })) || JSON.stringify(rows) !== JSON.stringify(rowsOf(loaded)))

  async function handleSave() {
    if (!draft || problems.length) return
    const result = await save.run({
      install: {
        timeoutSeconds: Number(draft.timeoutSeconds),
        chartRows: Number(draft.chartRows),
        mapPoints: Number(draft.mapPoints),
        studioRows: Number(draft.studioRows),
      },
      roles: rows.map((r) => ({ roleId: r.roleId, timeoutSeconds: Number(r.timeoutSeconds) })),
    })
    if (result) state.reload()
  }

  // What applies to the caller; the only thing a reader without settings:write is sent (BI-5 review S6).
  const forYou = loaded ? (
    <p className="text-xs text-muted-foreground" data-slot="effective-limits">
            For you: {loaded.effective.timeoutSeconds.toLocaleString("en-US")} seconds to run, {loaded.effective.chartRows.toLocaleString("en-US")} rows
            a chart reads, {loaded.effective.mapPoints.toLocaleString("en-US")} points on a map, {loaded.effective.studioRows.toLocaleString("en-US")} rows in
            Query Studio ({loaded.effective.source.kind === "role" ? `the limit for ${loaded.effective.source.name}` : "the limit for this install"}).
          </p>
  ) : null

  return (
    <section className="flex max-w-sm flex-col gap-4" aria-labelledby="settings-limits">
      <h2 id="settings-limits" className="text-sm font-medium">
        Query limits
      </h2>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {loaded && !draft ? forYou : null}
      {loaded && draft ? (
        <>
          {forYou}
          {FIELDS.map((field) => {
            const r = LIMIT_RANGES[field]
            const problem = limitProblem(field, draft[field])
            return (
              <div key={field} className="flex flex-col gap-1.5">
                <Label htmlFor={`limit-${field}`}>
                  {r.label} <span className="font-normal text-muted-foreground">({r.unit})</span>
                </Label>
                <Input
                  id={`limit-${field}`}
                  type="number"
                  inputMode="numeric"
                  min={r.min}
                  max={r.max}
                  value={draft[field]}
                  onChange={(e) => setDraft({ ...draft, [field]: e.target.value })}
                  disabled={!canWrite}
                  aria-invalid={problem ? true : undefined}
                />
                {problem ? <p role="alert" className="text-xs text-destructive">{problem}</p> : (
                  <p className="text-xs text-muted-foreground">
                    {r.min.toLocaleString("en-US")} to {r.max.toLocaleString("en-US")}.
                  </p>
                )}
              </div>
            )
          })}
          <div className="flex flex-col gap-2" role="group" aria-labelledby="limits-roles">
            <Label id="limits-roles">Time limit by role</Label>
            <p className="text-xs text-muted-foreground">
              A role listed here uses its own time limit instead of the install’s. A person with several roles gets the longest.
            </p>
            {rows.map((row, i) => {
              const problem = limitProblem("timeoutSeconds", row.timeoutSeconds)
              return (
                <div key={row.roleId} className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm">{nameOf(row.roleId)}</span>
                  <Input
                    type="number"
                    inputMode="numeric"
                    className="w-28"
                    aria-label={`Time limit for ${nameOf(row.roleId)} (seconds)`}
                    aria-invalid={problem ? true : undefined}
                    value={row.timeoutSeconds}
                    onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, timeoutSeconds: e.target.value } : r)))}
                    disabled={!canWrite}
                  />
                  {canWrite ? (
                    <Button variant="ghost" size="sm" aria-label={`Remove the limit for ${nameOf(row.roleId)}`} onClick={() => setRows(rows.filter((_, j) => j !== i))}>
                      Remove
                    </Button>
                  ) : null}
                </div>
              )
            })}
            {canWrite && free.length > 0 ? (
              <div className="flex items-center gap-2">
                <Select
                  value={pick}
                  items={Object.fromEntries(free.map((r) => [r.id, r.name]))}
                  onValueChange={(v) => setPick(v ?? "")}
                >
                  <SelectTrigger className="flex-1" aria-label="Role to give a time limit">
                    <SelectValue placeholder="Add a role" />
                  </SelectTrigger>
                  <SelectContent>
                    {free.map((r) => (
                      <SelectItem key={r.id} value={r.id}>
                        {r.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!pick}
                  onClick={() => {
                    setRows([...rows, { roleId: pick, timeoutSeconds: draft.timeoutSeconds }])
                    setPick("")
                  }}
                >
                  Add
                </Button>
              </div>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            A long time limit keeps a request open as long: with 30 minutes, a dashboard may take 30 minutes to answer. Exports keep their own limits.
          </p>
          {canWrite ? (
            <div className="flex items-center gap-3">
              <Button className="w-fit" onClick={handleSave} disabled={!changed || problems.length > 0 || save.status === "pending"}>
                Save limits
              </Button>
              {changed ? (
                <p role="status" className="text-xs text-amber-700 dark:text-amber-400">
                  Unsaved changes
                </p>
              ) : null}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">Only an administrator can change these.</p>
          )}
        </>
      ) : null}
    </section>
  )
}
