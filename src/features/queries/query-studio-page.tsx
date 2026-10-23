"use client"

import * as React from "react"

import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState } from "@/components/patterns/page-states"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { HistoryQuickList, SavedQuickList } from "./query-context-lists"
import { NaturalLanguagePanel } from "./nl-panel"
import { QueryResultsSection } from "./query-results-section"
import { QueryStudioTabs } from "./query-studio-tabs"
import { ExplainCard, FixCard, FixWithAiButton } from "./sql-help-card"
import { IcebergTimeTravelControls } from "./time-travel-controls"
import { QueryTransparencyPanel } from "./query-transparency-panel"
import { SaveQuerySheet } from "./save-query-sheet"
import { SaveSqlSourceSheet } from "./save-sql-source-sheet"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot } from "@/features/copilot/use-copilot"
import { summarizeQuery } from "@/lib/page-context-summary"
import { SchemaBrowser } from "./schema-browser"
import { SqlPanel } from "./sql-panel"
import { useQueryStudio } from "./use-query-studio"

/** Query Studio: natural-language ↔ SQL workspace with execution transparency. */
export function QueryStudioPage() {
  // All the state lives in the hook; this file is the layout.
  const studio = useQueryStudio()
  const [saveOpen, setSaveOpen] = React.useState(false)
  const [sourceOpen, setSourceOpen] = React.useState(false)
  const [schemaOpen, setSchemaOpen] = React.useState(false)
  const { hasPermission } = useAuth()
  const canAuthorSources = hasPermission("dashboard:sql")

  // Page-aware Copilot (plan §6): the SQL being written and the first rows
  // of its last result. Deferred so typing is not slowed by re-sending the
  // context on every keystroke.
  const { setPageContext } = useCopilot()
  const deferredSql = React.useDeferredValue(studio.sql)
  const lastResult = studio.runAct.data
  React.useEffect(() => {
    setPageContext({
      key: "query",
      title: "Write and run SQL",
      hint: "Ask about this query or its result, or have Copilot write SQL.",
      suggest: {
        ask: ["Explain this query", "What does this result show?", "Why might this query be slow?"],
        build: ["Turn this into a chart on a dashboard"],
      },
      system:
        "The user is in Query Studio. Prefer run_sql; offer to turn results into a chart.\n" +
        summarizeQuery(deferredSql, lastResult),
    })
    return () => setPageContext(null)
  }, [deferredSql, lastResult, setPageContext])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Query Studio"
        description="Ask questions or write SQL, then run it against the hot analytical store."
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
            <TabsContent value="sql" className="mt-3 space-y-3">
              {/* The browser sits beside the editor and never replaces it:
                  whatever the catalog answers, the editor keeps its width
                  and stays usable. */}
              <div
                className={
                  schemaOpen ? "grid gap-3 lg:grid-cols-[minmax(0,1fr)_300px]" : undefined
                }
              >
                <div className="min-w-0">
                  <SqlPanel
                    studio={studio}
                    onSave={() => setSaveOpen(true)}
                    onSaveAsSource={canAuthorSources ? () => setSourceOpen(true) : undefined}
                    onToggleSchema={() => setSchemaOpen((open) => !open)}
                    schemaOpen={schemaOpen}
                  />
                </div>
                {schemaOpen ? (
                  <SchemaBrowser studio={studio} onClose={() => setSchemaOpen(false)} />
                ) : null}
              </div>
              {/*
               * Iceberg time travel: pins one version of the raw tables
               * through a query setting. Shown whatever the engine, and
               * disabled with a note on Trino, so the reason is on screen
               * (DATA-16 F5).
               */}
              <IcebergTimeTravelControls
                sql={studio.sql}
                engine={studio.engine}
                onApply={studio.setSql}
              />
            </TabsContent>
          </Tabs>
          {studio.runAct.status === "error" ? (
            <div className="space-y-2">
              <ErrorState
                error={studio.runAct.error}
                onRetry={() => void studio.runQuery()}
              />
              {/*
               * Fix with AI works for ClickHouse only: the check that makes a
               * proposal safe to show runs as EXPLAIN PLAN, which Trino lacks.
               * Plan 16 lifts this. The reason is the button's title.
               */}
              <FixWithAiButton
                engine={studio.engine}
                busy={studio.fix.busy}
                onFix={() => void studio.fix.run(studio.runAct.error?.message ?? "")}
              />
            </div>
          ) : null}
          {studio.explain.error ? (
            <ErrorState error={studio.explain.error} />
          ) : studio.explain.result !== null ? (
            <ExplainCard text={studio.explain.result} />
          ) : null}
          {studio.fix.error ? (
            <ErrorState error={studio.fix.error} />
          ) : studio.fix.result ? (
            <FixCard
              outcome={studio.fix.result}
              editorSql={studio.sql}
              onAccept={studio.fix.accept}
              onReject={studio.fix.reject}
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
      {canAuthorSources ? (
        <SaveSqlSourceSheet open={sourceOpen} onOpenChange={setSourceOpen} sql={studio.sql} />
      ) : null}
    </div>
  )
}
