"use client"

import { PageHeader } from "@/components/patterns/page-header"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { useService } from "@/hooks/use-service"
import { formatNumber } from "@/lib/format"
import { identityService } from "@/services"
import type { WorkspaceSettings } from "@/services/contracts/identity"

const THEME_LABEL: Record<WorkspaceSettings["interfaceTheme"], string> = {
  dark: "Dark",
  light: "Light",
  system: "System",
}

function AdapterValue({
  adapter,
}: {
  adapter: WorkspaceSettings["serviceAdapter"]
}) {
  if (adapter === "mock") {
    return (
      <div>
        <p>Mock services</p>
        <p className="text-xs text-muted-foreground">
          No real backend is connected.
        </p>
      </div>
    )
  }
  return <span>HTTP services</span>
}

export function SettingsPage() {
  const state = useService((s) => identityService.getWorkspaceSettings(s), [])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Settings"
        description="Workspace defaults for environment, theme, and notification preferences."
      />
      {state.status === "loading" ? <LoadingSkeleton rows={4} /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <>
          <SectionCard title="Workspace">
            <MetadataList
              items={[
                { label: "Workspace name", value: state.data.workspaceName },
                {
                  label: "Default environment",
                  value: state.data.defaultEnvironment,
                },
                { label: "Default tenant", value: state.data.defaultTenant },
                {
                  label: "Interface theme",
                  value: THEME_LABEL[state.data.interfaceTheme],
                },
              ]}
            />
          </SectionCard>
          <SectionCard title="Data & retention">
            <MetadataList
              items={[
                {
                  label: "Service adapter",
                  value: <AdapterValue adapter={state.data.serviceAdapter} />,
                },
                {
                  label: "Audit retention",
                  value: `${formatNumber(state.data.auditRetentionDays)} days`,
                },
                {
                  label: "Query result retention",
                  value: `${formatNumber(state.data.queryResultRetentionDays)} days`,
                },
              ]}
            />
          </SectionCard>
        </>
      ) : null}
    </div>
  )
}
