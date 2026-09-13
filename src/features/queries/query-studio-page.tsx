"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { SqlEditor } from "@/components/sql-editor"
import { useService, useServiceAction } from "@/hooks/use-service"
import { insertAsOfClause } from "@/lib/snapshot-picker"
import { lakehouseService, queryService } from "@/services"
import { askAgentSql, type AgentQueryResult } from "@/services/clients/agent-client"
import type { QueryEngine } from "@/services/contracts/queries"
import { HistoryQuickList, SavedQuickList } from "./query-context-lists"
import { QueryResultsSection } from "./query-results-section"
import { QueryStudioTabs } from "./query-studio-tabs"

const STARTER_SQL = "-- Write SQL here, or generate it from a question"

/**
 * Iceberg time-travel controls: the user picks a namespace, then a table
 * (from `lakehouseService.listTables`), and that table's own snapshots
 * (from `getTableDetail`) feed the picker — the table is never inferred
 * from the SQL text itself. Only rendered when the engine is `trino`,
 * since `FOR VERSION AS OF` is meaningless against ClickHouse.
 */
function IcebergTimeTravelControls({
  sql,
  onApply,
}: {
  sql: string
  onApply: (nextSql: string) => void
}) {
  const [namespace, setNamespace] = React.useState<string | null>(null)
  const [table, setTable] = React.useState<string | null>(null)
  const [snapshotId, setSnapshotId] = React.useState<string | null>(null)

  const namespacesState = useService((s) => lakehouseService.listNamespaces(undefined, s), [])
  const tablesState = useService(
    (s) => (namespace ? lakehouseService.listTables(namespace, undefined, s) : Promise.resolve([])),
    [namespace]
  )
  const detailState = useService(
    (s) =>
      namespace && table
        ? lakehouseService.getTableDetail(namespace, table, s)
        : Promise.resolve(null),
    [namespace, table]
  )
  const snapshots = detailState.status === "success" && detailState.data ? detailState.data.snapshots : []

  function handleApply() {
    if (!namespace || !table || !snapshotId) return
    onApply(insertAsOfClause(sql, `${namespace}.${table}`, snapshotId))
  }

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md border border-dashed border-border p-2 text-xs">
      <span className="font-medium text-muted-foreground">Time travel</span>
      <Select
        value={namespace ?? ""}
        onValueChange={(v) => {
          setNamespace(v || null)
          setTable(null)
          setSnapshotId(null)
        }}
      >
        <SelectTrigger size="sm">
          <SelectValue placeholder="Namespace" />
        </SelectTrigger>
        <SelectContent>
          {namespacesState.status === "success"
            ? namespacesState.data.map((n) => (
                <SelectItem key={n.name} value={n.name}>
                  {n.name}
                </SelectItem>
              ))
            : null}
        </SelectContent>
      </Select>
      <Select
        value={table ?? ""}
        onValueChange={(v) => {
          setTable(v || null)
          setSnapshotId(null)
        }}
        disabled={!namespace}
      >
        <SelectTrigger size="sm">
          <SelectValue placeholder="Table" />
        </SelectTrigger>
        <SelectContent>
          {tablesState.status === "success"
            ? tablesState.data.map((t) => (
                <SelectItem key={t.name} value={t.name}>
                  {t.name}
                </SelectItem>
              ))
            : null}
        </SelectContent>
      </Select>
      <Select value={snapshotId ?? ""} onValueChange={(v) => setSnapshotId(v || null)} disabled={snapshots.length === 0}>
        <SelectTrigger size="sm">
          <SelectValue placeholder="Snapshot" />
        </SelectTrigger>
        <SelectContent>
          {snapshots.map((snap) => (
            <SelectItem key={snap.id} value={snap.id}>
              {snap.id} · {snap.operation}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button size="sm" variant="outline" onClick={handleApply} disabled={!namespace || !table || !snapshotId}>
        Insert AS OF
      </Button>
    </div>
  )
}

/** Query Studio: natural-language ↔ SQL workspace with execution transparency. */
export function QueryStudioPage() {
  const [tab, setTab] = React.useState("nl")
  const [question, setQuestion] = React.useState("")
  const [sql, setSql] = React.useState(STARTER_SQL)
  const [engine, setEngine] = React.useState<QueryEngine>("clickhouse")

  const generateAct = useServiceAction((signal, q: string) =>
    queryService.generateSql(q, signal)
  )
  const runAct = useServiceAction((signal, s: string, eng: QueryEngine) =>
    queryService.run(s, { engine: eng }, signal)
  )
  const savedState = useService((s) => queryService.listSaved(s), [])

  // Handoff from Saved Queries: /query-studio?saved=<id> loads that SQL.
  // Read from window.location to avoid a useSearchParams Suspense boundary.
  const appliedSavedRef = React.useRef(false)
  React.useEffect(() => {
    if (appliedSavedRef.current || savedState.status !== "success") return
    const savedId = new URLSearchParams(window.location.search).get("saved")
    if (!savedId) {
      appliedSavedRef.current = true
      return
    }
    const match = savedState.data.find((q) => q.id === savedId)
    if (match) {
      setSql(match.sql)
      setTab("sql")
    }
    appliedSavedRef.current = true
  }, [savedState])

  async function handleGenerate() {
    const out = await generateAct.run(question)
    if (out) {
      setSql(out.sql)
      setTab("sql")
    }
  }

  // Agentic ask: NL → generate SQL → RUN → self-correct on error →
  // explain the result. One button, the whole loop runs server-side (/api/agent/query).
  const [agentBusy, setAgentBusy] = React.useState(false)
  const [agentResult, setAgentResult] = React.useState<AgentQueryResult | null>(null)
  const [agentError, setAgentError] = React.useState<string | null>(null)
  async function handleAsk() {
    setAgentBusy(true)
    setAgentError(null)
    setAgentResult(null)
    try {
      const out = await askAgentSql(question)
      setAgentResult(out)
      setSql(out.sql) // load the final SQL into the editor for review/tweaking
    } catch (e) {
      setAgentError(e instanceof Error ? e.message : String(e))
    } finally {
      setAgentBusy(false)
    }
  }

  function loadSql(next: string) {
    setSql(next)
    setTab("sql")
  }

  const running = runAct.status === "pending"
  const generating = generateAct.status === "pending"

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Query Studio"
        description="Ask questions or write SQL, then run it against the hot analytical store."
      />
      <QueryStudioTabs />
      <div className="grid gap-4 xl:grid-cols-[1fr_320px]">
        <div className="min-w-0 space-y-4">
          <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
            <TabsList>
              <TabsTrigger value="nl">Natural language</TabsTrigger>
              <TabsTrigger value="sql">SQL</TabsTrigger>
            </TabsList>
            <TabsContent value="nl" className="mt-3 space-y-3">
              <Textarea
                value={question}
                onChange={(e) => setQuestion(e.target.value)}
                rows={4}
                placeholder="e.g. What was revenue by region last quarter?"
                aria-label="Natural language question"
              />
              <div className="flex items-center gap-3">
                <Button
                  size="sm"
                  onClick={handleAsk}
                  disabled={agentBusy || !question.trim()}
                >
                  {agentBusy ? "Agent working…" : "✦ Ask (agentic)"}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={handleGenerate}
                  disabled={generating || !question.trim()}
                >
                  {generating ? "Generating…" : "Generate SQL only"}
                </Button>
                {agentError ? (
                  <p className="text-xs text-destructive">{agentError}</p>
                ) : generateAct.status === "error" ? (
                  <p className="text-xs text-destructive">{generateAct.error.message}</p>
                ) : null}
              </div>

              {/* Agentic result: NL answer + step trace (plan→act→correct) + preview */}
              {agentResult ? (
                <SectionCard title="Agent answer">
                  <p className="text-sm">{agentResult.answer}</p>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {agentResult.rowCount} rows · final SQL loaded into the editor.
                  </p>
                  <details className="mt-3">
                    <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
                      Agent trace ({agentResult.steps.length} steps)
                    </summary>
                    <ol className="mt-2 space-y-1 text-xs text-muted-foreground">
                      {agentResult.steps.map((s, i) => (
                        <li key={i}>
                          <span className="font-mono text-foreground">{s.step}</span>: {s.detail}
                        </li>
                      ))}
                    </ol>
                  </details>
                  {agentResult.rows.length ? (
                    <div className="mt-3 overflow-x-auto rounded-md border">
                      <table className="w-full text-xs">
                        <thead>
                          <tr>
                            {agentResult.columns.map((c) => (
                              <th key={c} className="border-b px-2 py-1 text-left font-medium">{c}</th>
                            ))}
                          </tr>
                        </thead>
                        <tbody>
                          {agentResult.rows.slice(0, 10).map((r, i) => (
                            <tr key={i}>
                              {agentResult.columns.map((c) => (
                                <td key={c} className="border-b px-2 py-1 font-mono">
                                  {String((r as Record<string, unknown>)[c] ?? "")}
                                </td>
                              ))}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  ) : null}
                </SectionCard>
              ) : null}

              {generateAct.data ? (
                <SectionCard title="Explanation">
                  <p className="text-sm">{generateAct.data.explanation}</p>
                  {generateAct.data.assumptions.length ? (
                    <ul className="mt-2 list-disc pl-5 text-sm text-muted-foreground">
                      {generateAct.data.assumptions.map((a) => (
                        <li key={a}>{a}</li>
                      ))}
                    </ul>
                  ) : null}
                </SectionCard>
              ) : null}
            </TabsContent>
            <TabsContent value="sql" className="mt-3 space-y-3">
              <SqlEditor value={sql} onChange={setSql} />
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  onClick={() => void runAct.run(sql, engine)}
                  disabled={running || !sql.trim()}
                >
                  {running ? "Running…" : "Run query"}
                </Button>
                <Select value={engine} onValueChange={(v) => setEngine((v as QueryEngine) || "clickhouse")}>
                  <SelectTrigger size="sm">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="clickhouse">ClickHouse</SelectItem>
                    <SelectItem value="trino">Trino</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {engine === "trino" ? (
                <IcebergTimeTravelControls sql={sql} onApply={setSql} />
              ) : null}
            </TabsContent>
          </Tabs>
          {runAct.status === "error" ? (
            <ErrorState
              error={runAct.error}
              onRetry={() => void runAct.run(sql, engine)}
            />
          ) : null}
          {runAct.data ? <QueryResultsSection result={runAct.data} /> : null}
        </div>
        <div className="space-y-3">
          <SavedQuickList state={savedState} onLoadSql={loadSql} />
          <HistoryQuickList onLoadSql={loadSql} />
        </div>
      </div>
    </div>
  )
}
