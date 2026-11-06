"use client"

import * as React from "react"
import {
  CalendarClockIcon,
  GaugeIcon,
  HistoryIcon,
  LogInIcon,
  MailIcon,
  MapIcon,
  RocketIcon,
  SparklesIcon,
  TagsIcon,
} from "lucide-react"
import { LineTabsList, type LineTab } from "@/components/patterns/line-tabs"
import { PageHeader } from "@/components/patterns/page-header"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useUrlTab } from "@/hooks/use-url-tab"
import { useAuth } from "@/features/auth/auth-provider"
import { TagKeysPage } from "@/features/governance/tag-keys-page"
import { AssistantSettings } from "./assistant-settings"
import { DeploymentSettings } from "./deployment-settings"
import { EmailSettings } from "./email-settings"
import { MapsPage } from "./maps-page"
import { QueryLimitsSettings } from "./query-limits-settings"
import { RememberedResultsSettings } from "./remembered-results-settings"
import { ReportingSettingsPage } from "./reporting-settings-page"
import { SignInSettings } from "./sign-in-settings"
import { SsoPage } from "./sso-page"

export const SETTINGS_SECTIONS = [
  "reporting",
  "remembered-results",
  "query-limits",
  "sign-in",
  "tag-keys",
  "maps",
] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

/**
 * The sections only a holder of `settings:write` sees (`ADM-6`, `ADM-7`,
 * `ADM-9`): they name the deployment's own settings — the mail server, the
 * account, the console's address, the assistant's provider — which are not
 * every signed-in person's business.
 */
export const ADMIN_SETTINGS_SECTIONS = ["deployment", "email", "assistant"] as const
export type AdminSettingsSection = (typeof ADMIN_SETTINGS_SECTIONS)[number]

/** Every section of the page, administrator's or not. */
export type AnySettingsSection = SettingsSection | AdminSettingsSection

const TABS: LineTab[] = [
  { value: "reporting", label: "Reporting", icon: CalendarClockIcon },
  { value: "remembered-results", label: "Remembered results", icon: HistoryIcon },
  { value: "query-limits", label: "Query limits", icon: GaugeIcon },
  { value: "sign-in", label: "Sign-in", icon: LogInIcon },
  { value: "tag-keys", label: "Tag keys", icon: TagsIcon },
  { value: "maps", label: "Maps", icon: MapIcon },
]

const ADMIN_TABS: LineTab[] = [
  { value: "deployment", label: "Deployment", icon: RocketIcon },
  { value: "email", label: "Email", icon: MailIcon },
  { value: "assistant", label: "Assistant", icon: SparklesIcon },
]

/**
 * The sections of an administrator, in the order the strip shows them: the
 * deployment's own first (`ADM-6`, `ADM-7`), then everybody's.
 */
const ADMIN_SECTIONS: readonly AnySettingsSection[] = [
  ...ADMIN_SETTINGS_SECTIONS,
  ...SETTINGS_SECTIONS,
]

/**
 * Settings, one section at a time (`WEB-3` S3-D6). The sections were one
 * long page (reporting, remembered results, query limits, sign-in) plus
 * two pages of their own (Tag keys under Governance, Maps under
 * Administration); each is now a tab here, and the open one is in
 * `?section=` so `/governance/tag-keys` and `/admin/maps` can redirect to
 * theirs. Every section, not the whole page, loads when it is opened.
 *
 * The Deployment and Email sections (`ADM-6`, `ADM-7`) are only there for a
 * holder of `settings:write`: they name the deployment's own settings.
 */
export function SettingsPage() {
  const { hasPermission } = useAuth()
  const canAdminister = hasPermission("settings:write")
  const sections = canAdminister ? ADMIN_SECTIONS : SETTINGS_SECTIONS
  const tabs = canAdminister ? [...ADMIN_TABS, ...TABS] : TABS
  const [section, setSection] = useUrlTab<AnySettingsSection>("section", sections)
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Settings"
        description="How this deployment works for everyone: dates and reports, remembered results, query limits, sign-in, tag keys and maps."
      />
      <Tabs
        value={section}
        onValueChange={(v) => setSection(v as AnySettingsSection)}
        className="gap-4"
      >
        <LineTabsList tabs={tabs} active={section} />
        {canAdminister ? (
          <TabsContent value="deployment">
            <DeploymentSettings
              onOpenEmail={() => setSection("email")}
              onOpenAssistant={() => setSection("assistant")}
            />
          </TabsContent>
        ) : null}
        {canAdminister ? (
          <TabsContent value="email">
            <EmailSettings />
          </TabsContent>
        ) : null}
        {canAdminister ? (
          <TabsContent value="assistant">
            <AssistantSettings />
          </TabsContent>
        ) : null}
        <TabsContent value="reporting">
          <ReportingSettingsPage />
        </TabsContent>
        <TabsContent value="remembered-results">
          <RememberedResultsSettings />
        </TabsContent>
        <TabsContent value="query-limits">
          <QueryLimitsSettings />
        </TabsContent>
        <TabsContent value="sign-in">
          {/* `ADM-8`: the SSO status stays on top, and the deployment's own
              sign-in limits are below it — for a holder of `settings:write`,
              because the server refuses the read to anybody else. */}
          <div className="flex flex-col gap-6">
            <SsoPage />
            {canAdminister ? <SignInSettings /> : null}
          </div>
        </TabsContent>
        <TabsContent value="tag-keys">
          <TagKeysPage />
        </TabsContent>
        <TabsContent value="maps">
          <MapsPage />
        </TabsContent>
      </Tabs>
    </div>
  )
}
