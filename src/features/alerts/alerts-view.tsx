"use client"

import * as React from "react"
import { usePathname, useRouter, useSearchParams } from "next/navigation"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { PlatformAlertsPage } from "@/features/overview/alerts-page"
import { AlertRulesPage } from "@/features/alerts/alerts-page"

/**
 * BI-65 (owner QA): the tab is in the address, `/alerts?tab=rules`, so the
 * rule pages can come back to "Rules & Digests". No value, or any other,
 * reads as the incidents tab. A click replaces the address (no history entry
 * per tab, no scroll jump).
 */
export function AlertsView() {
  const router = useRouter()
  const pathname = usePathname()
  const searchParams = useSearchParams()
  const tab = searchParams.get("tab") === "rules" ? "rules" : "incidents"

  function onTabChange(next: unknown) {
    const params = new URLSearchParams(searchParams.toString())
    if (next === "rules") params.set("tab", "rules")
    else params.delete("tab")
    const query = params.toString()
    router.replace(query ? `${pathname}?${query}` : pathname, { scroll: false })
  }

  return (
    <div className="space-y-6">
      <Tabs value={tab} onValueChange={onTabChange}>
        <TabsList>
          <TabsTrigger value="incidents">Active Incidents</TabsTrigger>
          <TabsTrigger value="rules">Rules &amp; Digests</TabsTrigger>
        </TabsList>
        <TabsContent value="incidents" className="mt-4">
          <PlatformAlertsPage />
        </TabsContent>
        <TabsContent value="rules" className="mt-4">
          <AlertRulesPage />
        </TabsContent>
      </Tabs>
    </div>
  )
}
