"use client"

import { QueryLimitsSettings } from "@/features/admin/query-limits-settings"
import { RememberedResultsSettings } from "@/features/admin/remembered-results-settings"
import { ReportingSettingsPage } from "@/features/admin/reporting-settings-page"
import { SsoPage } from "@/features/admin/sso-page"

/**
 * Thin App Router page for the settings: reporting (BI-9, BI-17) the
 * remembered results (BI-4), the query limits (BI-5) and sign-in (WEB-3 D6).
 */
export default function Page() {
  return (
    <div className="flex flex-col gap-8">
      <ReportingSettingsPage />
      <RememberedResultsSettings />
      <QueryLimitsSettings />
      {/* WEB-3 D6: `/admin/sso` was one line and a button, so its content
          moved here as a "Sign-in" section. The component is reused as-is;
          it reads the live `GET /api/auth/providers` state. */}
      <SsoPage />
    </div>
  )
}
