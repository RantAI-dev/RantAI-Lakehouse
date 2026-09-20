"use client"

import * as React from "react"

import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState } from "@/components/patterns/page-states"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { HistoryQuickList, SavedQuickList } from "./query-context-lists"
import { NaturalLanguagePanel } from "./nl-panel"
import { QueryResultsSection } from "./query-results-section"
import { QueryStudioTabs } from "./query-studio-tabs"
import { QueryTransparencyPanel } from "./query-transparency-panel"
import { SaveQuerySheet } from "./save-query-sheet"
import { SqlPanel } from "./sql-panel"
import { useQueryStudio } from "./use-query-studio"

/** Query Studio: natural-language ↔ SQL workspace with execution transparency. */
export function QueryStudioPage() {
  // All the state lives in the hook; this file is the layout.
  const studio = useQueryStudio()
  const [saveOpen, setSaveOpen] = React.useState(false)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Query Studio"
        description="Ask questions or write SQL. Pre-run checks show workload class, engine category, freshness, policy obligations, and cost."
      />
      <QueryStudioTabs />
      <div className="grid gap-4 xl:grid-cols-[1fr_320px]">
        <div className="min-w-0 space-y-4">
          <Tabs
            value={studio.tab}
            onValueChange={(v) => studio.setTab(v === "sql" ? "sql" : "nl")}
          >
            <TabsList>
              <TabsTrigger value="nl">Natural language</TabsTrigger>
              <TabsTrigger value="sql">SQL</TabsTrigger>
            </TabsList>
            <TabsContent value="nl" className="mt-3">
              <NaturalLanguagePanel studio={studio} />
            </TabsContent>
            <TabsContent value="sql" className="mt-3">
              <SqlPanel studio={studio} onSave={() => setSaveOpen(true)} />
            </TabsContent>
          </Tabs>
          {studio.runAct.status === "error" ? (
            <ErrorState
              error={studio.runAct.error}
              onRetry={() => void studio.runQuery()}
            />
          ) : null}
          {studio.runAct.data ? (
            <QueryResultsSection result={studio.runAct.data} />
          ) : null}
        </div>
        <div className="space-y-3">
          <QueryTransparencyPanel
            state={studio.estimateAct}
            estimatable={studio.estimatable}
            onRetry={studio.reEstimate}
          />
          <SavedQuickList state={studio.savedState} onLoadSql={studio.loadSql} />
          <HistoryQuickList
            state={studio.historyState}
            onLoadSql={studio.loadSql}
          />
        </div>
      </div>

      <SaveQuerySheet
        open={saveOpen}
        onOpenChange={setSaveOpen}
        sql={studio.sql}
        saving={studio.saveAct.status === "pending"}
        error={studio.saveAct.error?.message ?? null}
        onSave={studio.save}
      />
    </div>
  )
}
