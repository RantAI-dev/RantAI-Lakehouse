"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import {
  settingsService,
  type DeploymentFeature,
  type DeploymentSettings as DeploymentSettingsData,
} from "@/services"

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
 * `BI-44`: the mode and engine the installer wrote, in the console's own
 * words. A value this build does not know is shown as the server sent it —
 * calling an unknown mode "full" would be a guess about the installation.
 */
export function installationModeLabel(mode?: string): string {
  if (mode === "full") return "Full lakehouse"
  if (mode === "bi-only") return "BI & Assistant only (in-place)"
  return mode ?? ""
}

/** `BI-44`: the configured query engine, widened the same way. */
export function deploymentEngineLabel(engine?: string): string {
  if (engine === "clickhouse") return "ClickHouse"
  if (engine === "trino") return "Trino"
  return engine ?? ""
}

/**
 * The installation summary (`BI-44`, `AC5`): what the installer chose, as
 * the deployment route reports it. It is CONFIGURATION, not a probe: a
 * service that was never installed is a different question, asked by the
 * Services view. Shown only when the server actually answered one of the
 * new fields, so an older server's `features`-only body draws no card.
 */
export function InstallationSummary({ deployment }: { deployment: DeploymentSettingsData }) {
  const { installMode, queryEngine, supportedInPlaceSources } = deployment
  if (installMode === undefined && queryEngine === undefined) return null
  return (
    <section aria-label="Installation summary" className="flex flex-col gap-2 rounded-lg border px-3 py-2">
      <h2 className="text-sm font-medium">Installation</h2>
      <dl className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
        {installMode === undefined ? null : (
          <div className="flex items-center gap-2">
            <dt className="text-muted-foreground">Installation mode</dt>
            <dd><Pill tone={installMode === "bi-only" ? "info" : "neutral"}>{installationModeLabel(installMode)}</Pill></dd>
          </div>
        )}
        {queryEngine === undefined ? null : (
          <div className="flex items-center gap-2">
            <dt className="text-muted-foreground">Query engine</dt>
            <dd><Pill tone="info">{deploymentEngineLabel(queryEngine)}</Pill></dd>
          </div>
        )}
      </dl>
      {installMode === "bi-only" && supportedInPlaceSources !== undefined ? (
        <div className="text-xs text-muted-foreground">
          <p>Data is read in place through the installed engine; nothing is copied into the lakehouse.</p>
          {supportedInPlaceSources.length > 0 ? (
            <ul aria-label="Supported external source types" className="mt-1 flex flex-wrap gap-1">
              {supportedInPlaceSources.map((source) => (
                <li key={source}>
                  <Pill tone="neutral">{source}</Pill>
                </li>
              ))}
            </ul>
          ) : (
            <p>No external source types are registered yet.</p>
          )}
        </div>
      ) : null}
    </section>
  )
}

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
  const deployment = state.status === "success" ? state.data : null
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
      {deployment ? <InstallationSummary deployment={deployment} /> : null}
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
