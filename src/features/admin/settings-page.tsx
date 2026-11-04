"use client"

import { CalendarClockIcon, GaugeIcon, HistoryIcon, LogInIcon, MapIcon, TagsIcon } from "lucide-react"
import { LineTabsList, type LineTab } from "@/components/patterns/line-tabs"
import { PageHeader } from "@/components/patterns/page-header"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useUrlTab } from "@/hooks/use-url-tab"
import { TagKeysPage } from "@/features/governance/tag-keys-page"
import { MapsPage } from "./maps-page"
import { QueryLimitsSettings } from "./query-limits-settings"
import { RememberedResultsSettings } from "./remembered-results-settings"
import { ReportingSettingsPage } from "./reporting-settings-page"
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

const TABS: LineTab[] = [
  { value: "reporting", label: "Reporting", icon: CalendarClockIcon },
  { value: "remembered-results", label: "Remembered results", icon: HistoryIcon },
  { value: "query-limits", label: "Query limits", icon: GaugeIcon },
  { value: "sign-in", label: "Sign-in", icon: LogInIcon },
  { value: "tag-keys", label: "Tag keys", icon: TagsIcon },
  { value: "maps", label: "Maps", icon: MapIcon },
]

/**
 * Settings, one section at a time (`WEB-3` S3-D6). The sections were one
 * long page (reporting, remembered results, query limits, sign-in) plus
 * two pages of their own (Tag keys under Governance, Maps under
 * Administration); each is now a tab here, and the open one is in
 * `?section=` so `/governance/tag-keys` and `/admin/maps` can redirect to
 * theirs. Every section is the component its old page rendered, unchanged
 * in what it reads and who may change it; only the section, not the whole
 * page, loads when it is opened.
 */
export function SettingsPage() {
  const [section, setSection] = useUrlTab("section", SETTINGS_SECTIONS)
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Settings"
        description="How this deployment works for everyone: dates and reports, remembered results, query limits, sign-in, tag keys and maps."
      />
      <Tabs value={section} onValueChange={(v) => setSection(v as SettingsSection)} className="gap-4">
        <LineTabsList tabs={TABS} active={section} />
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
          <SsoPage />
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
