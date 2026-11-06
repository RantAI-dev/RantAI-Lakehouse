"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { settingsService, type AssistantSettingsInput } from "@/services"
import { SettingField } from "./setting-field"

/**
 * Admin > Settings > Assistant (`ADM-9`): the provider the assistant
 * talks to, the model it uses and the key it authenticates with.
 *
 * The address, the model and the key are one unit: when the server's
 * environment holds a key, all three are the server's and are shown
 * read-only, so a key kept in files is never sent to an address chosen
 * here. Without a key, the address and the model answer for themselves:
 * an environment that sets `LLM_URL` owns the address alone, and one that
 * sets `LLM_MODEL` owns the model alone (`ADM-9 review R4`), so each
 * field is read-only on its own when the server owns it. The key is
 * write-only — the server never sends it back, so the field only says
 * whether one is set.
 */
export function AssistantSettings() {
  const { hasPermission } = useAuth()
  return <AssistantSettingsForm canWrite={hasPermission("settings:write")} />
}

export function AssistantSettingsForm({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => settingsService.getAssistant(signal), [])
  const save = useServiceAction(
    withNotify(
      { success: "Assistant settings saved", error: "Assistant settings were not saved" },
      (signal: AbortSignal, input: AssistantSettingsInput) =>
        settingsService.saveAssistant(input, signal)
    )
  )
  const test = useServiceAction((signal: AbortSignal) =>
    settingsService.testAssistant(signal)
  )
  const loaded = state.status === "success" ? state.data : null

  const [url, setUrl] = React.useState("")
  const [model, setModel] = React.useState("")
  const [key, setKey] = React.useState("")
  const [keyChoice, setKeyChoice] = React.useState<"keep" | "replace" | "clear">("keep")

  React.useEffect(() => {
    if (!loaded) return
    setUrl(loaded.url.value ?? "")
    setModel(loaded.model.value ?? "")
    setKey("")
    setKeyChoice("keep")
  }, [loaded])

  async function handleSave() {
    if (!loaded) return
    const input: AssistantSettingsInput = {}
    // A setting the server owns is never sent: the server would refuse the
    // whole save with a 409.
    if (loaded.url.source !== "server") {
      input.url = url.trim() === "" ? null : url.trim()
    }
    if (loaded.model.source !== "server") {
      input.model = model.trim() === "" ? null : model.trim()
    }
    if (loaded.key.source !== "server") {
      if (keyChoice === "replace" && key !== "") input.key = key
      if (keyChoice === "clear") input.key = null
    }
    const result = await save.run(input)
    if (result) {
      setKey("")
      setKeyChoice("keep")
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Assistant"
        description="The provider the assistant talks to, the model it uses and the key it authenticates with. The next message uses them, with no restart."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {loaded ? (
        <div className="flex max-w-sm flex-col gap-4">
          <SettingField label="Provider address" source={loaded.url.source} id="assistant-url">
            <Input
              id="assistant-url"
              aria-label="Provider address"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://api.example.com/v1"
              disabled={!canWrite || loaded.url.source === "server"}
              autoComplete="off"
            />
          </SettingField>
          <SettingField label="Model" source={loaded.model.source} id="assistant-model">
            <Input
              id="assistant-model"
              aria-label="Model"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="example-model"
              disabled={!canWrite || loaded.model.source === "server"}
              autoComplete="off"
            />
          </SettingField>
          <SettingField label="Key" source={loaded.key.source} id="assistant-key">
            {loaded.key.source === "server" ? null : keyChoice === "replace" ? (
              <>
                <Input
                  id="assistant-key"
                  aria-label="Key"
                  type="password"
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                  autoComplete="new-password"
                />
                <Button
                  variant="ghost"
                  size="sm"
                  className="w-fit"
                  onClick={() => {
                    setKey("")
                    setKeyChoice("keep")
                  }}
                >
                  Keep the saved key
                </Button>
              </>
            ) : (
              <>
                <p className="text-sm">{loaded.key.set ? "Set" : "Not set"}</p>
                {keyChoice === "clear" ? (
                  <p className="text-xs text-amber-700 dark:text-amber-400">
                    The saved key is cleared when you save.
                  </p>
                ) : null}
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setKeyChoice("replace")}
                  >
                    Replace
                  </Button>
                  {loaded.key.set ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setKeyChoice("clear")}
                    >
                      Clear
                    </Button>
                  ) : null}
                </div>
              </>
            )}
          </SettingField>
          {canWrite ? (
            <div className="flex items-center gap-3">
              <Button
                className="w-fit"
                onClick={handleSave}
                disabled={save.status === "pending"}
              >
                Save
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              Only an administrator can change these.
            </p>
          )}
          <section className="flex flex-col gap-2" aria-labelledby="assistant-test">
            <h2 id="assistant-test" className="text-sm font-medium">
              Test the provider
            </h2>
            <p className="text-xs text-muted-foreground">
              One short request with the saved settings.
            </p>
            <div className="flex items-center gap-3">
              <Button
                variant="outline"
                className="w-fit"
                onClick={() => test.run()}
                disabled={test.status === "pending"}
              >
                Test the provider
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
