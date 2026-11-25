"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { settingsService, type DeploymentFeature } from "@/services"

/**
 * The configuration guide in the public repository: where the settings only
 * the server owns are written down (`docs/CONFIGURATION.md`).
 */
export const CONFIGURATION_DOC =
  "https://github.com/RantAI-dev/RantAI-Lakehouse/blob/main/docs/CONFIGURATION.md"

/** What each feature is called in the console. */
export const FEATURE_LABEL: Record<DeploymentFeature["feature"], string> = {
  pdfExport: "PDF export",
  email: "Email",
  consoleAddress: "Console address",
  singleSignOn: "Single sign-on",
  assistant: "Assistant",
}

/**
 * The features the console can fix itself. Everything else is set in the
 * server's environment, so the row points at the guide.
 */
const FIXED_IN_THE_CONSOLE: DeploymentFeature["feature"][] = ["email", "assistant"]

/**
 * Admin > Settings > Deployment (`ADM-6`): which of the deployment's
 * features are set up, and what is missing for the ones that are not.
 *
 * It reports CONFIGURATION, never health: a feature is set up when its
 * settings are in place. Whether the mail server really answers is the
 * Email tab's question, not this page's.
 */
export function DeploymentSettings({
  onOpenEmail,
  onOpenAssistant,
}: {
  onOpenEmail?: () => void
  onOpenAssistant?: () => void
}) {
  const state = useService((signal) => settingsService.getDeployment(signal), [])
  const loaded = state.status === "success" ? state.data.features : []
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Deployment"
        description="Which of this deployment's features are set up. This is the configuration, not a health check: a feature is set up when its settings are in place."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <ul className="flex flex-col gap-2" aria-label="Deployment features">
          {loaded.map((feature) => (
            <li
              key={feature.feature}
              className="flex flex-wrap items-center gap-3 rounded-lg border px-3 py-2"
            >
              <span className="text-sm font-medium">{FEATURE_LABEL[feature.feature]}</span>
              <Pill tone={feature.on ? "success" : "warning"}>
                {feature.on ? "Set up" : "Not set up"}
              </Pill>
              {feature.missing.length > 0 ? (
                <span className="text-xs text-muted-foreground">
                  Missing: {feature.missing.join(", ")}
                </span>
              ) : null}
              {feature.on || FIXED_IN_THE_CONSOLE.indexOf(feature.feature) < 0 ? null : (
                <FixLink
                  feature={feature}
                  onOpenEmail={onOpenEmail}
                  onOpenAssistant={onOpenAssistant}
                />
              )}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

/**
 * Where to fix a feature that is not set up: the tab that owns the setting
 * when the console owns it, the configuration guide when the server does.
 */
function FixLink({
  feature,
  onOpenEmail,
  onOpenAssistant,
}: {
  feature: DeploymentFeature
  onOpenEmail?: () => void
  onOpenAssistant?: () => void
}) {
  if (feature.where === "console") {
    if (feature.feature === "email" && onOpenEmail) {
      return (
        <Button variant="outline" size="sm" onClick={onOpenEmail}>
          Set it up in the Email tab
        </Button>
      )
    }
    if (feature.feature === "assistant" && onOpenAssistant) {
      return (
        <Button variant="outline" size="sm" onClick={onOpenAssistant}>
          Set it up in the Assistant tab
        </Button>
      )
    }
  }
  return (
    <a
      className="text-xs underline underline-offset-4"
      href={CONFIGURATION_DOC}
      target="_blank"
      rel="noreferrer"
    >
      How to set it up
    </a>
  )
}
