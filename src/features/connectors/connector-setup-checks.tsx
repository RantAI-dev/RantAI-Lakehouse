"use client"

import { CopyIcon } from "lucide-react"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { orderedChecks } from "@/lib/connectors/capture-lag"
import { notifyError, notifySuccess } from "@/lib/notify"
import type { SetupCheck } from "@/services/contracts/connectors"

const STATUS_PILL: Record<string, { label: string; tone: "success" | "warning" | "neutral" | "destructive" }> = {
  ok: { label: "OK", tone: "success" },
  missing: { label: "Missing", tone: "destructive" },
  unchecked: { label: "Not checked", tone: "neutral" },
}

async function copyCommand(command: string) {
  try {
    await navigator.clipboard.writeText(command)
    notifySuccess("Command copied")
  } catch (err) {
    // Usually a denied clipboard permission or a non-secure origin.
    notifyError("Failed to copy the command", err)
  }
}

/**
 * What a change-capture source must have switched on (`SRC-4` decision S4-f), the missing settings first, each with
 * the command that sets it and a Copy button. The words are the API's fixed text; a setting that could not be read
 * says so and is never shown as passing. The connector can be saved whatever this says.
 */
export function ConnectorSetupChecks({ checks }: { checks: readonly SetupCheck[] }) {
  if (checks.length === 0) return null
  const missing = checks.filter((c) => c.status === "missing").length
  return (
    <section aria-label="Source setup" className="space-y-2 rounded-lg border border-border bg-muted/20 px-3 py-2">
      <p className="text-sm font-medium">
        Source setup
        <span className="ml-2 text-xs font-normal text-muted-foreground">
          {missing === 0 ? "Nothing is missing." : `${missing} setting${missing === 1 ? "" : "s"} missing. You can still save the connector.`}
        </span>
      </p>
      <ul className="space-y-2">
        {orderedChecks(checks).map((check) => {
          const pill = STATUS_PILL[check.status] ?? { label: check.status, tone: "neutral" as const }
          return (
            <li key={check.setting} className="space-y-1">
              <div className="flex flex-wrap items-center gap-2">
                <Pill tone={pill.tone}>{pill.label}</Pill>
                <span className="text-sm font-medium">{check.setting}</span>
              </div>
              <p className="text-xs text-muted-foreground">{check.message}</p>
              {check.command ? (
                <div className="flex items-start gap-2">
                  <code className="min-w-0 flex-1 break-all rounded bg-muted px-2 py-1 font-mono text-xs">
                    {check.command}
                  </code>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    aria-label={`Copy the command for ${check.setting}`}
                    onClick={() => void copyCommand(check.command ?? "")}
                  >
                    <CopyIcon data-icon="inline-start" />
                    Copy
                  </Button>
                </div>
              ) : null}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
