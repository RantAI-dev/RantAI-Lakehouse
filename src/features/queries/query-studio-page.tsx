"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { SqlEditor } from "@/components/sql-editor"
import { useService, useServiceAction } from "@/hooks/use-service"
import { assetService, queryService } from "@/services"
import { askAgentSql, type AgentQueryResult } from "@/services/clients/agent-client"
import { HistoryQuickList, SavedQuickList } from "./query-context-lists"
import { QueryResultsSection } from "./query-results-section"
import { QueryStudioTabs } from "./query-studio-tabs"
import { QueryTransparencyPanel } from "./query-transparency-panel"

const STARTER_SQL = "-- Write SQL here, or generate it from a question"

/** Query Studio: natural-language ↔ SQL workspace with execution transparency. */
export function QueryStudioPage() {
  const [tab, setTab] = React.useState("nl")
  const [question, setQuestion] = React.useState("")
  const [sql, setSql] = React.useState(STARTER_SQL)

  const generateAct = useServiceAction((signal, q: string) =>
    queryService.generateSql(q, signal)
  )
  const estimateAct = useServiceAction((signal, s: string) =>
    queryService.estimate(s, signal)
  )
  const runAct = useServiceAction((signal, s: string) =>
    queryService.run(s, signal)
  )
  const savedState = useService((s) => queryService.listSaved(s), [])

  // Skema untuk autocomplete editor, diambil dari katalog nyata supaya saran
  // tidak melenceng saat katalog berubah.
  //
  // Hanya NAMA TABEL yang dilengkapi, bukan kolomnya: daftar kolom hanya ada
  // di `AssetDetail`, dan mengambilnya berarti satu request per aset — biaya
  // yang tidak sepadan hanya untuk melengkapi saran. Kolom bisa menyusul bila
  // backend menyediakan endpoint skema massal.
  //
  // Kegagalan di sini sengaja tidak ditampilkan sebagai error halaman:
  // autocomplete adalah penyempurna, dan Query Studio tetap berguna tanpanya.
  const assetsState = useService((s) => assetService.listAssets({}, s), [])
  const sqlSchema = React.useMemo(() => {
    if (assetsState.status !== "success") return undefined
    const schema: Record<string, string[]> = {}
    for (const asset of assetsState.data) {
      schema[`${asset.namespace}.${asset.name}`] = []
    }
    return Object.keys(schema).length > 0 ? schema : undefined
  }, [assetsState.status, assetsState.data])

  const runEstimate = estimateAct.run
  React.useEffect(() => {
    if (sql.trim()) void runEstimate(sql)
  }, [sql, runEstimate])

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

  // Agentic ask: NL → generate SQL → JALANKAN → koreksi diri bila error →
  // jelaskan hasil. Satu tombol, loop penuh di server (/api/agent/query).
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
      setSql(out.sql) // muat SQL final ke editor untuk ditinjau/di-tweak
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
        description="Ask questions or write SQL. Pre-run checks show workload class, engine category, freshness, policy obligations, and cost."
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
                  {agentBusy ? "Agent bekerja…" : "✦ Ask (agentic)"}
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

              {/* Hasil agentic: jawaban NL + jejak langkah (plan→act→correct) + preview */}
              {agentResult ? (
                <SectionCard title="Jawaban agent">
                  <p className="text-sm">{agentResult.answer}</p>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {agentResult.rowCount} baris · SQL final dimuat ke editor.
                  </p>
                  <details className="mt-3">
                    <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
                      Jejak agent ({agentResult.steps.length} langkah)
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
              <SqlEditor value={sql} onChange={setSql} schema={sqlSchema} />
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  onClick={() => void runAct.run(sql)}
                  disabled={running || !sql.trim()}
                >
                  {running ? "Running…" : "Run query"}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void runEstimate(sql)}
                  disabled={estimateAct.status === "pending" || !sql.trim()}
                >
                  Estimate
                </Button>
              </div>
            </TabsContent>
          </Tabs>
          {runAct.status === "error" ? (
            <ErrorState
              error={runAct.error}
              onRetry={() => void runAct.run(sql)}
            />
          ) : null}
          {runAct.data ? <QueryResultsSection result={runAct.data} /> : null}
        </div>
        <div className="space-y-3">
          <QueryTransparencyPanel
            state={estimateAct}
            onRetry={() => void runEstimate(sql)}
          />
          <SavedQuickList state={savedState} onLoadSql={loadSql} />
          <HistoryQuickList onLoadSql={loadSql} />
        </div>
      </div>
    </div>
  )
}
