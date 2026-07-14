"use client"

import { useCallback, useMemo, useState } from "react"
import {
  ChevronRight,
  Cpu,
  Database,
  Layers,
  Loader2,
  ScanSearch,
  Sparkles,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

const TAB_SEARCH = "search"
const TAB_SIMILARITY = "similarity"

type Hit = {
  id: string
  title: string
  zone: string
  score: number
  snippet: string
}

const SAMPLE_SEMANTIC_HITS: Hit[] = [
  {
    id: "t-gold-orders",
    title: "gold.orders_fact",
    zone: "Gold",
    score: 0.91,
    snippet:
      "Fact table for orders — semantic match on revenue and fulfillment windows.",
  },
  {
    id: "t-silver-cust",
    title: "silver.dim_customer",
    zone: "Silver",
    score: 0.84,
    snippet:
      "Customer attributes used downstream for segmentation and LTV features.",
  },
  {
    id: "t-semantic-vec",
    title: "semantic.support_embeddings",
    zone: "Semantic",
    score: 0.78,
    snippet:
      "Vector index over ticket text — nearest neighbors for similar issues.",
  },
]

type ZoneFilter = "all" | "Gold" | "Silver" | "Semantic"
const ZONE_CHIPS: { id: ZoneFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "Gold", label: "Gold" },
  { id: "Silver", label: "Silver" },
  { id: "Semantic", label: "Semantic" },
]

type SimZoneFilter = ZoneFilter
const SIM_ZONE_CHIPS: { id: SimZoneFilter; label: string }[] = [
  { id: "all", label: "All zones" },
  { id: "Gold", label: "Gold" },
  { id: "Silver", label: "Silver" },
  { id: "Semantic", label: "Semantic" },
]

type Neighbor = {
  id: string
  label: string
  similarity: number
  zone: string
  snippet: string
  sourceTable: string
  recordId: string
  updatedAt: string
  /** Short hint for why this row surfaced (mock) */
  matchHint: string
}

type AnchorMeta = {
  id: string
  label: string
  subtitle: string
  recordId: string
  zone: string
  sourceTable: string
  previewText: string
  indexName: string
  embeddingModel: string
  embeddingDims: number
  metric: "cosine"
}

const ANCHORS: AnchorMeta[] = [
  {
    id: "a1",
    label: "Ticket #4821 — login timeout",
    subtitle: "Support · opened 2h ago",
    recordId: "tck_4821",
    zone: "Semantic",
    sourceTable: "semantic.support_tickets",
    previewText:
      "User reports repeated login timeout after SSO redirect; session cookie appears cleared mid-flow. Reproduced on Chrome 131.",
    indexName: "idx_support_tickets_v3",
    embeddingModel: "text-embedding-3-large",
    embeddingDims: 1536,
    metric: "cosine",
  },
  {
    id: "a2",
    label: "Order O-90014 — high value",
    subtitle: "Sales · Gold",
    recordId: "ord_90014",
    zone: "Gold",
    sourceTable: "gold.orders_fact",
    previewText:
      "High-value retail order with bundled wealth product; same customer segment as Q4 uplift campaign.",
    indexName: "idx_orders_semantic_v1",
    embeddingModel: "text-embedding-3-large",
    embeddingDims: 1536,
    metric: "cosine",
  },
  {
    id: "a3",
    label: "Branch Jakarta — throughput",
    subtitle: "Analytics · weekly",
    recordId: "br_jkt_w12",
    zone: "Gold",
    sourceTable: "gold.branch_metrics_weekly",
    previewText:
      "Weekly branch throughput vs target; staffing and queue time drivers for Jakarta central district.",
    indexName: "idx_branch_narrative_v2",
    embeddingModel: "text-embedding-3-large",
    embeddingDims: 1536,
    metric: "cosine",
  },
]

const NEIGHBORS: Record<string, Neighbor[]> = {
  a1: [
    {
      id: "n1",
      label: "Ticket #4799 — SSO redirect loop",
      similarity: 0.93,
      zone: "Semantic",
      snippet:
        "Customer stuck in redirect loop between IdP and app; same SSO error code as reference.",
      sourceTable: "semantic.support_tickets",
      recordId: "tck_4799",
      updatedAt: "2h ago",
      matchHint: "Shared token / SSO failure phrases in body text",
    },
    {
      id: "n2",
      label: "Ticket #4702 — session expiry",
      similarity: 0.88,
      zone: "Semantic",
      snippet:
        "Session drops unexpectedly after idle; user must re-auth — overlaps with timeout semantics.",
      sourceTable: "semantic.support_tickets",
      recordId: "tck_4702",
      updatedAt: "1d ago",
      matchHint: "Session + authentication vocabulary overlap",
    },
    {
      id: "n3",
      label: "Incident INC-221 — auth latency",
      similarity: 0.81,
      zone: "Gold",
      snippet:
        "P1 incident: elevated auth latency region-wide; correlated with gateway saturation.",
      sourceTable: "gold.incidents_dim",
      recordId: "inc_221",
      updatedAt: "3d ago",
      matchHint: "Cross-zone link via incident text embedding bridge",
    },
    {
      id: "n10",
      label: "Ticket #4610 — password reset loop",
      similarity: 0.76,
      zone: "Semantic",
      snippet:
        "User cannot complete password reset; similar auth flow complaints.",
      sourceTable: "semantic.support_tickets",
      recordId: "tck_4610",
      updatedAt: "5d ago",
      matchHint: "Lower lexical overlap; still in auth cluster",
    },
  ],
  a2: [
    {
      id: "n4",
      label: "Order O-8992 — same customer",
      similarity: 0.9,
      zone: "Gold",
      snippet:
        "Same customer_id and product bundle as reference; prior quarter repeat purchase.",
      sourceTable: "gold.orders_fact",
      recordId: "ord_8992",
      updatedAt: "4h ago",
      matchHint: "Customer + product embedding neighborhood",
    },
    {
      id: "n5",
      label: "Order O-90013 — portfolio cross-sell",
      similarity: 0.85,
      zone: "Gold",
      snippet:
        "Cross-sell motion on wealth SKU; narrative aligned with reference order notes.",
      sourceTable: "gold.orders_fact",
      recordId: "ord_90013",
      updatedAt: "1d ago",
      matchHint: "Campaign text similarity on order comments",
    },
    {
      id: "n6",
      label: "Campaign C-12 — conversion",
      similarity: 0.72,
      zone: "Silver",
      snippet:
        "Campaign metadata describing uplift segment; weaker but same semantic cluster.",
      sourceTable: "silver.campaign_dim",
      recordId: "cmp_12",
      updatedAt: "2d ago",
      matchHint: "Indirect via campaign description embedding",
    },
  ],
  a3: [
    {
      id: "n7",
      label: "Branch Bandung — throughput YoY",
      similarity: 0.89,
      zone: "Gold",
      snippet:
        "YoY throughput narrative for Bandung region; same metric family as Jakarta.",
      sourceTable: "gold.branch_metrics_weekly",
      recordId: "br_bdg_w12",
      updatedAt: "6h ago",
      matchHint: "Regional weekly report template similarity",
    },
    {
      id: "n8",
      label: "Branch Surabaya — accounts opened",
      similarity: 0.84,
      zone: "Silver",
      snippet:
        "Weekly branch summary emphasizing new accounts; comparable operational story.",
      sourceTable: "silver.branch_ops_weekly",
      recordId: "br_sby_w12",
      updatedAt: "1d ago",
      matchHint: "Shared branch KPI vocabulary",
    },
    {
      id: "n9",
      label: "Regional rollup Q1",
      similarity: 0.77,
      zone: "Gold",
      snippet:
        "Quarterly rollup including Jakarta cluster; broader scope, lower similarity.",
      sourceTable: "gold.regional_rollups",
      recordId: "reg_q1_26",
      updatedAt: "3d ago",
      matchHint: "Parent geography in same embedding space",
    },
  ],
}

export default function SemanticSearchPage() {
  const [tab, setTab] = useState(TAB_SEARCH)

  const [q, setQ] = useState("")
  const [topK, setTopK] = useState("10")
  const [zoneFilter, setZoneFilter] = useState<ZoneFilter>("all")
  const [loading, setLoading] = useState(false)
  const [hits, setHits] = useState<Hit[] | null>(null)

  const [anchor, setAnchor] = useState(ANCHORS[0]!.id)
  const [anchorListQuery, setAnchorListQuery] = useState("")
  const [simTopK, setSimTopK] = useState("5")
  const [simZoneFilter, setSimZoneFilter] = useState<SimZoneFilter>("all")

  const neighborsRaw = useMemo(() => NEIGHBORS[anchor] ?? [], [anchor])

  const neighbors = useMemo(() => {
    let list = neighborsRaw
    if (simZoneFilter !== "all") {
      list = list.filter((n) => n.zone === simZoneFilter)
    }
    const k = Math.min(Number(simTopK) || 5, 20)
    return list.slice(0, k)
  }, [neighborsRaw, simZoneFilter, simTopK])

  const filteredAnchors = useMemo(() => {
    const s = anchorListQuery.trim().toLowerCase()
    if (!s) return ANCHORS
    return ANCHORS.filter(
      (a) =>
        a.label.toLowerCase().includes(s) ||
        a.subtitle.toLowerCase().includes(s)
    )
  }, [anchorListQuery])

  const displayHits = useMemo(() => {
    if (!hits) return null
    if (zoneFilter === "all") return hits
    return hits.filter((h) => h.zone === zoneFilter)
  }, [hits, zoneFilter])

  const run = useCallback(() => {
    if (!q.trim()) return
    setLoading(true)
    setHits(null)
    const k = Math.min(Number(topK) || 10, SAMPLE_SEMANTIC_HITS.length)
    window.setTimeout(() => {
      setHits(SAMPLE_SEMANTIC_HITS.slice(0, k))
      setLoading(false)
    }, 650)
  }, [q, topK])

  const selectedAnchor = ANCHORS.find((a) => a.id === anchor)

  const simStats = useMemo(() => {
    const above80 = neighborsRaw.filter((n) => n.similarity >= 0.8).length
    return { above80, total: neighborsRaw.length }
  }, [neighborsRaw])

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-6">
      <div className="text-center sm:text-left">
        <h1 className="text-2xl font-semibold tracking-tight text-primary">
          Search &amp; similarity
        </h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Search tables and assets by meaning, or find items similar to a
          reference record.
        </p>
      </div>

      <Tabs value={tab} onValueChange={setTab} className="w-full">
        <TabsList className="grid h-11 w-full grid-cols-2 rounded-lg bg-muted/60 p-1 sm:max-w-md">
          <TabsTrigger
            value={TAB_SEARCH}
            className="gap-2 rounded-md data-[state=active]:bg-background data-[state=active]:shadow-sm"
          >
            <ScanSearch className="size-4" />
            Search
          </TabsTrigger>
          <TabsTrigger
            value={TAB_SIMILARITY}
            className="gap-2 rounded-md data-[state=active]:bg-background data-[state=active]:shadow-sm"
          >
            <Sparkles className="size-4" />
            Find similar
          </TabsTrigger>
        </TabsList>

        {/* —— Semantic search (common “search bar + filters + result cards”) —— */}
        <TabsContent value={TAB_SEARCH} className="mt-6 space-y-5 outline-none">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <Label htmlFor="semantic-q" className="sr-only">
              Search query
            </Label>
            <div className="relative flex-1">
              <ScanSearch
                className="pointer-events-none absolute left-3 top-1/2 size-5 -translate-y-1/2 text-muted-foreground"
                aria-hidden
              />
              <Input
                id="semantic-q"
                value={q}
                onChange={(e) => setQ(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") run()
                }}
                placeholder="Describe what you are looking for…"
                className="h-12 rounded-lg border-border bg-background pl-11 pr-24 text-base shadow-sm"
                autoComplete="off"
              />
              <Button
                type="button"
                size="sm"
                className="absolute right-1.5 top-1/2 h-9 -translate-y-1/2 px-4"
                onClick={run}
                disabled={loading || !q.trim()}
              >
                {loading ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden />
                ) : (
                  "Search"
                )}
              </Button>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <span className="whitespace-nowrap text-sm text-muted-foreground">
                Top
              </span>
              <Select
                value={topK}
                onValueChange={(v) => {
                  if (v) setTopK(v)
                }}
              >
                <SelectTrigger className="h-9 w-[88px] rounded-md border-border">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="5">5</SelectItem>
                  <SelectItem value="10">10</SelectItem>
                  <SelectItem value="15">15</SelectItem>
                  <SelectItem value="20">20</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Zone
            </span>
            {ZONE_CHIPS.map((z) => (
              <Button
                key={z.id}
                type="button"
                variant={zoneFilter === z.id ? "default" : "outline"}
                size="sm"
                className={cn(
                  "h-8 rounded-full px-3 text-xs",
                  zoneFilter === z.id && "pointer-events-none"
                )}
                onClick={() => setZoneFilter(z.id)}
              >
                {z.label}
              </Button>
            ))}
          </div>

          <Separator />

          {loading && (
            <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
              <Loader2 className="size-5 animate-spin text-primary" />
              Searching…
            </div>
          )}

          {!loading && !hits && (
            <p className="py-10 text-center text-sm text-muted-foreground">
              Enter a query and press Search to see ranked results.
            </p>
          )}

          {!loading && displayHits && displayHits.length === 0 && (
            <p className="py-10 text-center text-sm text-muted-foreground">
              No results in this zone. Try another filter or query.
            </p>
          )}

          {!loading && displayHits && displayHits.length > 0 && (
            <ul className="flex flex-col gap-1">
              {displayHits.map((h, i) => (
                <li key={h.id}>
                  <button
                    type="button"
                    className="w-full rounded-lg border border-transparent px-3 py-3 text-left transition-colors hover:border-border hover:bg-muted/40"
                  >
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <div className="min-w-0 flex-1">
                        <p className="font-mono text-sm font-semibold text-primary">
                          {h.title}
                        </p>
                        <p className="mt-1 line-clamp-2 text-sm leading-snug text-muted-foreground">
                          {h.snippet}
                        </p>
                      </div>
                      <div className="flex shrink-0 flex-col items-end gap-1">
                        <span className="text-sm font-semibold tabular-nums text-foreground">
                          {Math.round(h.score * 100)}
                          <span className="text-muted-foreground">%</span>
                        </span>
                        <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                          match
                        </span>
                      </div>
                    </div>
                    <div className="mt-2 flex items-center gap-2">
                      <Badge
                        variant="secondary"
                        className="text-[11px] font-normal"
                      >
                        {h.zone}
                      </Badge>
                      <span className="text-xs text-muted-foreground">
                        Result {i + 1}
                      </span>
                    </div>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </TabsContent>

        {/* —— Similarity: reference record + ranked neighbors with rich metadata —— */}
        <TabsContent value={TAB_SIMILARITY} className="mt-6 outline-none">
          <div className="grid gap-6 rounded-xl border border-border bg-card p-4 shadow-sm sm:p-6 xl:grid-cols-[minmax(0,280px)_1fr]">
            <div className="flex min-h-0 flex-col gap-3">
              <div>
                <h2 className="text-sm font-semibold text-foreground">
                  Reference item
                </h2>
                <p className="text-xs text-muted-foreground">
                  Pick the row whose embedding is used as the query vector.
                </p>
              </div>
              <Input
                value={anchorListQuery}
                onChange={(e) => setAnchorListQuery(e.target.value)}
                placeholder="Filter references…"
                className="h-9"
              />
              <ul
                className="max-h-[min(320px,45vh)] overflow-y-auto rounded-md border border-border"
                role="listbox"
              >
                {filteredAnchors.map((a) => (
                  <li key={a.id}>
                    <button
                      type="button"
                      role="option"
                      aria-selected={anchor === a.id}
                      className={cn(
                        "flex w-full flex-col gap-0.5 border-b border-border px-3 py-2.5 text-left text-sm transition-colors last:border-b-0",
                        anchor === a.id
                          ? "bg-primary/10 text-primary"
                          : "hover:bg-muted/60"
                      )}
                      onClick={() => setAnchor(a.id)}
                    >
                      <span className="font-medium leading-tight">{a.label}</span>
                      <span className="text-xs text-muted-foreground">
                        {a.subtitle}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>

            <div className="flex min-h-0 min-w-0 flex-col gap-5">
              {selectedAnchor && (
                <Card className="gap-0 overflow-hidden py-0 ring-border/80">
                  <CardHeader className="border-b border-border bg-muted/25 px-4 py-3 sm:px-5">
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <div className="min-w-0 space-y-1">
                        <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                          Embedding source
                        </p>
                        <CardTitle className="text-base font-semibold leading-snug">
                          {selectedAnchor.label}
                        </CardTitle>
                        <p className="font-mono text-[11px] text-muted-foreground">
                          {selectedAnchor.recordId} · {selectedAnchor.sourceTable}
                        </p>
                      </div>
                      <Badge variant="secondary" className="shrink-0 text-[11px]">
                        {selectedAnchor.zone}
                      </Badge>
                    </div>
                  </CardHeader>
                  <CardContent className="space-y-4 px-4 py-4 sm:px-5">
                    <p className="text-sm leading-relaxed text-foreground">
                      {selectedAnchor.previewText}
                    </p>
                    <div className="grid gap-3 rounded-lg border border-border/80 bg-muted/20 p-3 sm:grid-cols-2">
                      <div className="flex gap-2">
                        <Layers
                          className="mt-0.5 size-4 shrink-0 text-primary"
                          aria-hidden
                        />
                        <div>
                          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                            Vector index
                          </p>
                          <p className="font-mono text-xs text-foreground">
                            {selectedAnchor.indexName}
                          </p>
                        </div>
                      </div>
                      <div className="flex gap-2">
                        <Cpu
                          className="mt-0.5 size-4 shrink-0 text-primary"
                          aria-hidden
                        />
                        <div>
                          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                            Model · dims
                          </p>
                          <p className="text-xs text-foreground">
                            {selectedAnchor.embeddingModel} ·{" "}
                            {selectedAnchor.embeddingDims}d
                          </p>
                        </div>
                      </div>
                      <div className="flex gap-2 sm:col-span-2">
                        <Database
                          className="mt-0.5 size-4 shrink-0 text-primary"
                          aria-hidden
                        />
                        <div>
                          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                            Distance metric
                          </p>
                          <p className="text-xs capitalize text-foreground">
                            {selectedAnchor.metric} (higher = closer)
                          </p>
                        </div>
                      </div>
                    </div>
                  </CardContent>
                </Card>
              )}

              <div className="space-y-3">
                <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
                  <div>
                    <h2 className="text-sm font-semibold text-foreground">
                      Nearest neighbors
                    </h2>
                    {selectedAnchor && (
                      <p className="mt-0.5 text-xs text-muted-foreground">
                        Ranked by {selectedAnchor.metric} similarity to this
                        record&apos;s embedding.
                      </p>
                    )}
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Top</span>
                    <Select
                      value={simTopK}
                      onValueChange={(v) => {
                        if (v) setSimTopK(v)
                      }}
                    >
                      <SelectTrigger className="h-8 w-[72px] rounded-md border-border text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="3">3</SelectItem>
                        <SelectItem value="5">5</SelectItem>
                        <SelectItem value="10">10</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                    Zone
                  </span>
                  {SIM_ZONE_CHIPS.map((z) => (
                    <Button
                      key={z.id}
                      type="button"
                      variant={simZoneFilter === z.id ? "default" : "outline"}
                      size="sm"
                      className={cn(
                        "h-7 rounded-full px-2.5 text-[11px]",
                        simZoneFilter === z.id && "pointer-events-none"
                      )}
                      onClick={() => setSimZoneFilter(z.id)}
                    >
                      {z.label}
                    </Button>
                  ))}
                </div>

                <p className="rounded-md border border-dashed border-border/80 bg-muted/15 px-3 py-2 text-[11px] text-muted-foreground">
                  <span className="font-medium text-foreground">
                    {neighbors.length}
                  </span>{" "}
                  neighbor{neighbors.length === 1 ? "" : "s"} shown
                  {simZoneFilter !== "all" ? ` in ${simZoneFilter}` : ""} ·{" "}
                  <span className="font-medium text-foreground">
                    {simStats.above80}
                  </span>{" "}
                  of {simStats.total} above 80% in full list
                </p>
              </div>

              <ul className="flex flex-col gap-3">
                {neighbors.length === 0 ? (
                  <li className="rounded-lg border border-border bg-muted/20 px-4 py-8 text-center text-sm text-muted-foreground">
                    No neighbors in this zone. Choose &quot;All zones&quot; or
                    another reference.
                  </li>
                ) : (
                  neighbors.map((n, idx) => (
                    <li key={n.id}>
                      <div className="overflow-hidden rounded-lg border border-border bg-background shadow-sm">
                        <div className="flex flex-col gap-3 border-b border-border/80 px-4 py-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
                          <div className="min-w-0 flex-1 space-y-2">
                            <div className="flex flex-wrap items-center gap-2">
                              <span className="text-[10px] font-medium tabular-nums text-muted-foreground">
                                #{idx + 1}
                              </span>
                              <Badge
                                variant="secondary"
                                className="text-[10px] font-normal"
                              >
                                {n.zone}
                              </Badge>
                              <span className="font-mono text-[10px] text-muted-foreground">
                                {n.recordId}
                              </span>
                            </div>
                            <p className="text-sm font-semibold leading-snug text-foreground">
                              {n.label}
                            </p>
                            <p className="text-sm leading-relaxed text-muted-foreground">
                              {n.snippet}
                            </p>
                            <p className="rounded-md bg-primary/5 px-2.5 py-1.5 text-[11px] leading-snug text-primary">
                              <span className="font-medium">Why it matches: </span>
                              {n.matchHint}
                            </p>
                          </div>
                          <div className="flex shrink-0 flex-row items-center justify-between gap-4 sm:flex-col sm:items-end sm:justify-start">
                            <div className="text-right sm:text-right">
                              <span className="text-2xl font-bold tabular-nums tracking-tight text-primary">
                                {Math.round(n.similarity * 100)}
                                <span className="text-base font-semibold text-muted-foreground">
                                  %
                                </span>
                              </span>
                              <p className="text-[10px] uppercase text-muted-foreground">
                                cosine
                              </p>
                            </div>
                            <div className="text-right text-[11px] text-muted-foreground">
                              <span className="block font-mono">
                                dist ≈ {(1 - n.similarity).toFixed(3)}
                              </span>
                              <span className="text-[10px]">1 − similarity</span>
                            </div>
                          </div>
                        </div>
                        <div className="grid gap-3 px-4 py-3 sm:grid-cols-2">
                          <div>
                            <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                              Source table
                            </p>
                            <p className="mt-0.5 font-mono text-xs text-foreground">
                              {n.sourceTable}
                            </p>
                          </div>
                          <div>
                            <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                              Updated
                            </p>
                            <p className="mt-0.5 text-xs text-foreground">
                              {n.updatedAt}
                            </p>
                          </div>
                        </div>
                        <div className="px-4 pb-3">
                          <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                            <div
                              className="h-full rounded-full bg-primary/75 transition-[width]"
                              style={{ width: `${n.similarity * 100}%` }}
                            />
                          </div>
                        </div>
                      </div>
                    </li>
                  ))
                )}
              </ul>

              <p className="flex items-start gap-2 text-xs text-muted-foreground">
                <ChevronRight className="mt-0.5 size-3.5 shrink-0" aria-hidden />
                <span>
                  Neighbors are illustrative. In production, scores come from your
                  vector index (e.g. pgvector, OpenSearch kNN); open a row in Query
                  Studio or the data catalog to validate lineage and freshness.
                </span>
              </p>
            </div>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
