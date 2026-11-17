"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { settingsService, type EmailSettingsInput } from "@/services"
import { SettingField } from "./setting-field"

/** The two ways the connection to the mail server may be secured. */
const SECURITY = { tls: "Implicit TLS", starttls: "STARTTLS" } as const

/**
 * Admin > Settings > Email (`ADM-7`): the mail server, the account it
 * sends with, the password and the console's public address.
 *
 * A setting the server's environment owns is shown read-only, with the
 * reason: the server's value wins (plan D1), and the console may not
 * change it. The password is write-only — the server never sends it back,
 * so the field only says whether one is set.
 */
export function EmailSettings() {
  const { hasPermission } = useAuth()
  return <EmailSettingsForm canWrite={hasPermission("settings:write")} />
}

export function EmailSettingsForm({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => settingsService.getEmail(signal), [])
  const save = useServiceAction(
    withNotify(
      { success: "Email settings saved", error: "Email settings were not saved" },
      (signal: AbortSignal, input: EmailSettingsInput) =>
        settingsService.saveEmail(input, signal)
    )
  )
  const test = useServiceAction((signal: AbortSignal) =>
    settingsService.sendTestEmail(signal)
  )
  const loaded = state.status === "success" ? state.data : null

  const [host, setHost] = React.useState("")
  const [port, setPort] = React.useState("")
  const [security, setSecurity] = React.useState<"tls" | "starttls">("starttls")
  const [user, setUser] = React.useState("")
  const [from, setFrom] = React.useState("")
  const [consoleUrl, setConsoleUrl] = React.useState("")
  const [password, setPassword] = React.useState("")
  const [passwordChoice, setPasswordChoice] = React.useState<
    "keep" | "replace" | "clear"
  >("keep")

  React.useEffect(() => {
    if (!loaded) return
    setHost(loaded.host.value ?? "")
    setPort(loaded.port.value === null ? "" : String(loaded.port.value))
    setSecurity(loaded.security.value ?? "starttls")
    setUser(loaded.user.value ?? "")
    setFrom(loaded.from.value ?? "")
    setConsoleUrl(loaded.consolePublicUrl.value ?? "")
    setPassword("")
    setPasswordChoice("keep")
  }, [loaded])

  const portNumber = port.trim() === "" ? null : Number(port)
  const portValid =
    port.trim() === "" ||
    (Number.isInteger(portNumber) && portNumber !== null && portNumber >= 1 && portNumber <= 65535)

  async function handleSave() {
    if (!loaded) return
    const input: EmailSettingsInput = {}
    // A setting the server owns is never sent: the server would refuse the
    // whole save with a 409.
    if (loaded.host.source !== "server") {
      input.host = host.trim() === "" ? null : host.trim()
    }
    if (loaded.port.source !== "server" && portNumber !== null && portValid) {
      input.port = portNumber
    }
    if (loaded.security.source !== "server") {
      input.security = security
    }
    if (loaded.user.source !== "server") {
      input.user = user.trim() === "" ? null : user.trim()
    }
    if (loaded.from.source !== "server") {
      input.from = from.trim() === "" ? null : from.trim()
    }
    if (loaded.consolePublicUrl.source !== "server") {
      input.consolePublicUrl = consoleUrl.trim() === "" ? null : consoleUrl.trim()
    }
    if (loaded.password.source !== "server") {
      if (passwordChoice === "replace" && password !== "") input.password = password
      if (passwordChoice === "clear") input.password = null
    }
    const result = await save.run(input)
    if (result) {
      setPassword("")
      setPasswordChoice("keep")
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Email"
        description="The mail server this deployment sends with, and the console's public address. The next send uses it, with no restart."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {loaded ? (
        <div className="flex max-w-sm flex-col gap-4">
          <SettingField label="Mail server" source={loaded.host.source} id="email-host">
            <Input
              id="email-host"
              aria-label="Mail server"
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="smtp.example.com"
              disabled={!canWrite || loaded.host.source === "server"}
              autoComplete="off"
            />
          </SettingField>
          <SettingField label="Port" source={loaded.port.source} id="email-port">
            <Input
              id="email-port"
              aria-label="Port"
              type="number"
              min={1}
              max={65535}
              value={port}
              onChange={(e) => setPort(e.target.value)}
              placeholder="587"
              disabled={!canWrite || loaded.port.source === "server"}
              autoComplete="off"
            />
            {portValid ? null : (
              <p role="alert" className="text-xs text-destructive">
                The port must be a whole number from 1 to 65535.
              </p>
            )}
          </SettingField>
          <SettingField label="Security" source={loaded.security.source} id="email-security">
            <Select
              value={security}
              items={SECURITY}
              onValueChange={(v) => setSecurity((v as "tls" | "starttls") ?? security)}
              disabled={!canWrite || loaded.security.source === "server"}
            >
              <SelectTrigger id="email-security" className="w-full" aria-label="Security">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.entries(SECURITY).map(([value, label]) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SettingField>
          <SettingField label="User name" source={loaded.user.source} id="email-user">
            <Input
              id="email-user"
              aria-label="User name"
              value={user}
              onChange={(e) => setUser(e.target.value)}
              placeholder="reports"
              disabled={!canWrite || loaded.user.source === "server"}
              autoComplete="off"
            />
          </SettingField>
          <SettingField label="Password" source={loaded.password.source} id="email-password">
            {loaded.password.source === "server" ? null : passwordChoice === "replace" ? (
              <>
                <Input
                  id="email-password"
                  aria-label="Password"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete="new-password"
                />
                <Button
                  variant="ghost"
                  size="sm"
                  className="w-fit"
                  onClick={() => {
                    setPassword("")
                    setPasswordChoice("keep")
                  }}
                >
                  Keep the saved password
                </Button>
              </>
            ) : (
              <>
                <p className="text-sm">{loaded.password.set ? "Set" : "Not set"}</p>
                {passwordChoice === "clear" ? (
                  <p className="text-xs text-amber-700 dark:text-amber-400">
                    The saved password is cleared when you save.
                  </p>
                ) : null}
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPasswordChoice("replace")}
                  >
                    Replace
                  </Button>
                  {loaded.password.set ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setPasswordChoice("clear")}
                    >
                      Clear
                    </Button>
                  ) : null}
                </div>
              </>
            )}
          </SettingField>
          <SettingField label="From address" source={loaded.from.source} id="email-from">
            <Input
              id="email-from"
              aria-label="From address"
              value={from}
              onChange={(e) => setFrom(e.target.value)}
              placeholder="reports@example.com"
              disabled={!canWrite || loaded.from.source === "server"}
              autoComplete="off"
            />
          </SettingField>
          <SettingField
            label="Console address"
            source={loaded.consolePublicUrl.source}
            id="email-console-url"
          >
            <Input
              id="email-console-url"
              aria-label="Console address"
              value={consoleUrl}
              onChange={(e) => setConsoleUrl(e.target.value)}
              placeholder="https://console.example.com"
              disabled={!canWrite || loaded.consolePublicUrl.source === "server"}
              autoComplete="off"
            />
          </SettingField>
          {canWrite ? (
            <div className="flex items-center gap-3">
              <Button
                className="w-fit"
                onClick={handleSave}
                disabled={!portValid || save.status === "pending"}
              >
                Save
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              Only an administrator can change these.
            </p>
          )}
          <section className="flex flex-col gap-2" aria-labelledby="email-test">
            <h2 id="email-test" className="text-sm font-medium">
              Test email
            </h2>
            <p className="text-xs text-muted-foreground">
              One short email to your own account address, with the saved settings.
            </p>
            <div className="flex items-center gap-3">
              <Button
                variant="outline"
                className="w-fit"
                onClick={() => test.run()}
                disabled={test.status === "pending"}
              >
                Send test email
              </Button>
              {test.status === "success" && test.data ? (
                <p role="status" className="text-sm">
                  {test.data.message}
                </p>
              ) : null}
            </div>
          </section>
        </div>
      ) : null}
    </div>
  )
}
