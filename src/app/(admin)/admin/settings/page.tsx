"use client"

import { QueryLimitsSettings } from "@/features/admin/query-limits-settings"
import { RememberedResultsSettings } from "@/features/admin/remembered-results-settings"
import { ReportingSettingsPage } from "@/features/admin/reporting-settings-page"

/** Thin App Router page for the settings: reporting (BI-9, BI-17) the remembered results (BI-4) and the query limits (BI-5). */
export default function Page() {
  return (
    <div className="flex flex-col gap-8">
      <ReportingSettingsPage />
      <RememberedResultsSettings />
      <QueryLimitsSettings />
    </div>
  )
}
