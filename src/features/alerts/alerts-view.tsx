"use client"

import * as React from "react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { PlatformAlertsPage } from "@/features/overview/alerts-page"
import { AlertRulesPage } from "@/features/alerts/alerts-page"

export function AlertsView() {
  return (
    <div className="space-y-6">
      <Tabs defaultValue="incidents">
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
