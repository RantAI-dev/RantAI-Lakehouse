"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { settingsService, type SignInSettingsInput } from "@/services"
import { SettingField } from "./setting-field"

/**
 * The names the server knows the five limits by.
 */
type SignInField =
  | "maxFailures"
  | "failureWindowSecs"
  | "lockoutSecs"
  | "retentionDays"
  | "embedMaxLifetimeSecs"

/**
 * One limit of the form: its field name, its label, the range the server
 * accepts and the sentence it refuses a value outside that range with.
 */
type Limit = {
  /** The field name the server knows it by. */
  field: SignInField
  /** The label on the form. */
  label: string
  /** The smallest value the server accepts. */
  min: number
  /** The largest value the server accepts. */
  max: number
  /** What the server says when a value is outside the range. */
  sentence: string
}

/** The five limits plan D12 names, with the ranges it gives them. */
const LIMITS: Limit[] = [
  {
    field: "maxFailures",
    label: "Failed sign-ins before the lock",
    min: 3,
    max: 20,
    sentence: "maxFailures must be a whole number from 3 to 20.",
  },
  {
    field: "failureWindowSecs",
    label: "Counting window (seconds)",
    min: 60,
    max: 86400,
    sentence: "failureWindowSecs must be a whole number from 60 to 86400.",
  },
  {
    field: "lockoutSecs",
    label: "Lockout (seconds)",
    min: 60,
    max: 86400,
    sentence: "lockoutSecs must be a whole number from 60 to 86400.",
  },
  {
    field: "retentionDays",
    label: "Sign-in records kept (days)",
    min: 7,
    max: 3650,
    sentence: "retentionDays must be a whole number from 7 to 3650.",
  },
  {
    field: "embedMaxLifetimeSecs",
    label: "Longest embed link (seconds)",
    min: 300,
    max: 86400,
    sentence: "embedMaxLifetimeSecs must be a whole number from 300 to 86400.",
  },
]

/**
 * Admin > Settings > Sign-in (`ADM-8`): the deployment's sign-in limits.
 *
 * A limit the server's environment owns is shown read-only, with the
 * reason: the server's value wins (plan D1), and the console may not
 * change it. The next sign-in uses what is saved here, with no restart.
 */
export function SignInSettings() {
  const { hasPermission } = useAuth()
  return <SignInSettingsForm canWrite={hasPermission("settings:write")} />
}

export function SignInSettingsForm({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => settingsService.getSignIn(signal), [])
  const save = useServiceAction(
    withNotify(
      { success: "Sign-in settings saved", error: "Sign-in settings were not saved" },
      (signal: AbortSignal, input: SignInSettingsInput) =>
        settingsService.saveSignIn(input, signal)
    )
  )
  const loaded = state.status === "success" ? state.data : null
  const [values, setValues] = React.useState<Record<string, string>>({})

  React.useEffect(() => {
    if (!loaded) return
    const next: Record<string, string> = {}
    for (const limit of LIMITS) {
      const value = loaded[limit.field].value
      next[limit.field] = value === null ? "" : String(value)
    }
    setValues(next)
  }, [loaded])

  /** The value the field holds, or `null` when it is not a whole number in range. */
  function numberIn(limit: Limit): number | null {
    const text = (values[limit.field] ?? "").trim()
    if (text === "") return null
    const number = Number(text)
    if (!Number.isInteger(number)) return null
    if (number < limit.min || number > limit.max) return null
    return number
  }

  const invalid = LIMITS.filter((limit) => {
    const text = (values[limit.field] ?? "").trim()
    if (text === "") return false
    return numberIn(limit) === null
  })

  async function handleSave() {
    if (!loaded) return
    const input: SignInSettingsInput = {}
    // A limit the server owns is never sent: the server would refuse the
    // whole save with a 409.
    for (const limit of LIMITS) {
      if (loaded[limit.field].source === "server") continue
      const number = numberIn(limit)
      if (number !== null) input[limit.field] = number
    }
    const result = await save.run(input)
    if (result) state.reload()
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Sign-in limits"
        description="How many wrong passwords a person may try before the lock, how long the lock lasts, how long sign-in records are kept and how long an embed link may live. The next sign-in uses them, with no restart."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {loaded ? (
        <div className="flex max-w-sm flex-col gap-4">
          {LIMITS.map((limit) => {
            const source = loaded[limit.field].source
            const text = values[limit.field] ?? ""
            const invalidHere = invalid.includes(limit)
            return (
              <SettingField key={limit.field} label={limit.label} source={source} id={`sign-in-${limit.field}`}>
                <Input
                  id={`sign-in-${limit.field}`}
                  aria-label={limit.label}
                  type="number"
                  min={limit.min}
                  max={limit.max}
                  value={text}
                  onChange={(e) =>
                    setValues((current) => ({ ...current, [limit.field]: e.target.value }))
                  }
                  disabled={!canWrite || source === "server"}
                  autoComplete="off"
                />
                {invalidHere ? (
                  <p role="alert" className="text-xs text-destructive">
                    {limit.sentence}
                  </p>
                ) : null}
              </SettingField>
            )
          })}
          {canWrite ? (
            <div className="flex items-center gap-3">
              <Button
                className="w-fit"
                onClick={handleSave}
                disabled={invalid.length > 0 || save.status === "pending"}
              >
                Save
              </Button>
            </div>
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
