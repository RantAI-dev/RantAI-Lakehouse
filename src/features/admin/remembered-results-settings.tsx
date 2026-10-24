"use client"

import * as React from "react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { FreshnessField } from "@/features/dashboards/freshness-field"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { draftOf, draftProblem, ruleOf, type RuleDraft } from "@/lib/result-cache"
import { withNotify } from "@/lib/notify"
import { settingsService } from "@/services"

/**
 * Admin > Settings > Remembered results (`BI-4` part B, feature decisions 1, 2
 * and 7): how long dashboards may show remembered results for the whole
 * install: follow the default (until the data under a chart changes, and never
 * longer than 24 hours), always fresh, or a number of minutes. A dashboard or a
 * chart can set its own. Anyone signed in sees the rule; only a holder of
 * `settings:write` changes it (the server enforces it; the controls are
 * disabled here so the page does not offer what would be refused).
 *
 * It saves through the reporting settings route, sending the zone and first
 * day it just read and only the rule, so nothing else on that screen changes.
 */
export function RememberedResultsSettings() {
  const { hasPermission } = useAuth()
  return <RememberedResultsForm canWrite={hasPermission("settings:write")} />
}

export function RememberedResultsForm({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => settingsService.getReporting(signal), [])
  const [draft, setDraft] = React.useState<RuleDraft>(draftOf(null))
  const save = useServiceAction(
    withNotify({ success: "Remembered results setting saved", error: "The setting was not saved" }, (signal, rule: { mode: string; minutes?: number }) =>
      settingsService.saveReporting(
        { timeZone: state.status === "success" ? state.data.timeZone : "UTC", weekStart: state.status === "success" ? state.data.weekStart : "monday", resultCache: rule },
        signal
      )
    )
  )
  const loaded = state.status === "success" ? state.data : null
  React.useEffect(() => {
    if (loaded) setDraft(draftOf(loaded.resultCache))
  }, [loaded])

  const saved = draftOf(loaded?.resultCache)
  const changed = loaded !== null && (draft.mode !== saved.mode || (draft.mode === "minutes" && draft.minutes !== saved.minutes))
  const problem = draftProblem(draft)

  async function handleSave() {
    if (problem) return
    const result = await save.run(ruleOf(draft))
    if (result) state.reload()
  }

  return (
    <section className="flex max-w-sm flex-col gap-4" aria-labelledby="settings-remembered">
      <h2 id="settings-remembered" className="text-sm font-medium">
        Remembered results
      </h2>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {loaded ? (
        <>
          <FreshnessField
            id="install-freshness"
            label="Remember dashboard results"
            draft={draft}
            onChange={setDraft}
            disabled={!canWrite}
            help="Follow the default: a result is used until the data under the chart changes, and never longer than 24 hours (10 minutes for data the engine cannot watch). A dashboard or a chart can choose its own."
          />
          {canWrite ? (
            <div className="flex items-center gap-3">
              <Button className="w-fit" onClick={handleSave} disabled={!changed || problem !== null || save.status === "pending"}>
                Save
              </Button>
              {changed ? (
                <p role="status" className="text-xs text-amber-700 dark:text-amber-400">
                  Unsaved changes
                </p>
              ) : null}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">Only an administrator can change this.</p>
          )}
        </>
      ) : null}
    </section>
  )
}
