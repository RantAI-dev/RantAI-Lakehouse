"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useServiceAction } from "@/hooks/use-service"
import { knowledgeService } from "@/services"
import type { SearchStrategy } from "@/services/contracts/knowledge"

export function SemanticSearchPage() {
  const [query, setQuery] = React.useState("late payment escalation")
  const [strategy, setStrategy] = React.useState<SearchStrategy>("hybrid")
  const search = useServiceAction((signal, q: string, s: SearchStrategy) =>
    knowledgeService.search(q, s, signal)
  )
  const lastRunRef = React.useRef<{ query: string; strategy: SearchStrategy } | null>(
    null
  )

  const runSearch = (q: string, s: SearchStrategy) => {
    lastRunRef.current = { query: q, strategy: s }
    void search.run(q, s)
  }

  const onStrategyChange = (next: SearchStrategy) => {
    setStrategy(next)
    if (lastRunRef.current) runSearch(query, next)
  }

  const pending = search.status === "pending"

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Semantic Search"
        description="Semantic, lexical, and hybrid retrieval with citations, version, and freshness."
        actions={
          <Button size="sm" onClick={() => runSearch(query, strategy)} disabled={pending}>
            {pending ? "Searching…" : "Search"}
          </Button>
        }
      />
      <div className="flex flex-wrap gap-2">
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") runSearch(query, strategy)
          }}
          className="max-w-md"
          aria-label="Search query"
        />
        <Tabs value={strategy} onValueChange={(v) => onStrategyChange(v as SearchStrategy)}>
          <TabsList>
            <TabsTrigger value="semantic">Semantic</TabsTrigger>
            <TabsTrigger value="lexical">Lexical</TabsTrigger>
            <TabsTrigger value="hybrid">Hybrid</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      {search.status === "idle" ? (
        <EmptyState
          title="Search the knowledge base"
          description="Run a query and compare semantic, lexical, and hybrid retrieval strategies."
        />
      ) : null}
      {search.status === "pending" ? <LoadingSkeleton rows={4} /> : null}
      {search.status === "error" ? (
        <ErrorState
          error={search.error}
          onRetry={() => {
            const last = lastRunRef.current
            if (last) runSearch(last.query, last.strategy)
          }}
        />
      ) : null}
      {search.status === "success" && search.data.length === 0 ? (
        <EmptyState
          title="No results"
          description="No knowledge chunks matched this query. Try a different phrasing or strategy."
        />
      ) : null}
      {search.status === "success" && search.data.length > 0 ? (
        <div className="space-y-3">
          {search.data.map((h) => (
            <SectionCard
              key={h.id}
              title={h.title}
              description={`${Math.round(h.score * 100)}% match`}
              action={<Pill tone="neutral">{h.strategy}</Pill>}
            >
              <p className="text-sm">{h.snippet}</p>
              <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                <span className="font-mono">{h.source}</span>
                <span>{h.version}</span>
                <FreshnessIndicator lagSeconds={h.freshnessLagSeconds} />
              </div>
            </SectionCard>
          ))}
        </div>
      ) : null}
    </div>
  )
}
