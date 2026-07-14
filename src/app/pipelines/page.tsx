"use client"

import { Suspense, useCallback, useEffect, useMemo, useState } from "react"
import Link from "next/link"
import { usePathname, useRouter, useSearchParams } from "next/navigation"
import {
  ArrowRight,
  BrainCircuit,
  Clock,
  Database,
  Eye,
  EyeOff,
  History,
  Loader2,
  Play,
  Plus,
  RefreshCcw,
  Save,
  Sparkles,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { AgenticPipelineBuilderDialog } from "@/components/pipelines/agentic-pipeline-builder-dialog"
import { SAMPLE_PIPELINES } from "@/lib/pipeline-fixtures"
import { cn } from "@/lib/utils"
import type { PipelineItem } from "@/types/pipeline"

const TAB_DATA = "data"
const TAB_VECTOR = "vector"

const SUMMARY_DATA = [
  { label: "Total pipelines", value: 142 },
  { label: "Healthy", value: 100 },
  { label: "Running", value: 20 },
  { label: "Failed", value: 22 },
] as const

const SUMMARY_VECTOR = [
  { label: "Vector jobs", value: 6 },
  { label: "Healthy", value: 5 },
  { label: "Running", value: 1 },
  { label: "Index size", value: "4.2 GB" },
] as const

interface RunHistoryRow {
  id: string
  started: string
  duration: string
  status: PipelineItem["status"]
  records: string
}

const SAMPLE_RUN_HISTORY: RunHistoryRow[] = [
  {
    id: "run-9821",
    started: "2026-04-12 09:40:02",
    duration: "3m 12s",
    status: "Success",
    records: "125k",
  },
  {
    id: "run-9814",
    started: "2026-04-12 09:15:00",
    duration: "3m 05s",
    status: "Success",
    records: "124k",
  },
  {
    id: "run-9799",
    started: "2026-04-12 08:50:44",
    duration: "2m 48s",
    status: "Failed",
    records: "0",
  },
  {
    id: "run-9782",
    started: "2026-04-12 08:25:01",
    duration: "3m 01s",
    status: "Success",
    records: "123k",
  },
]

export type EmbeddingJob = {
  id: string
  name: string
  enabled: boolean
  model: string
  sourceColumn: string
  targetIndex: string
  chunkTokens: string
  cron: string
  notes: string
  status: "Idle" | "Running" | "Failed"
  lastRun: string
  lastDuration: string
  vectorsUpserted: string
  indexSize: string
}

const INITIAL_EMBEDDING_JOBS: EmbeddingJob[] = [
  {
    id: "emb-1",
    name: "Support tickets — description",
    enabled: true,
    model: "text-embedding-3-small",
    sourceColumn: "gold.tickets_fact.description",
    targetIndex: "semantic.support_embeddings",
    chunkTokens: "512",
    cron: "0 */4 * * *",
    notes: "PII: strip emails before embedding.",
    status: "Idle",
    lastRun: "2026-04-12 08:00",
    lastDuration: "3m 18s",
    vectorsUpserted: "12,402",
    indexSize: "842 MB",
  },
  {
    id: "emb-2",
    name: "Product catalog — attributes",
    enabled: true,
    model: "text-embedding-3-large",
    sourceColumn: "silver.dim_product.attributes_json",
    targetIndex: "semantic.product_vectors",
    chunkTokens: "256",
    cron: "0 2 * * *",
    notes: "",
    status: "Idle",
    lastRun: "2026-04-11 02:05",
    lastDuration: "8m 02s",
    vectorsUpserted: "89,100",
    indexSize: "2.1 GB",
  },
]

/** Generates a unique id for a newly-created vector / embedding job. */
function newEmbeddingJobId(): string {
  return `emb-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`
}

/** Colored pill that shows pipeline run status (Success / Failed / Running). */
function StatusBadge({ status }: { status: PipelineItem["status"] }) {
  const variant =
    status === "Success"
      ? "bg-[#ecfdf2] text-[#008a2e] border-0 hover:bg-[#ecfdf2]"
      : status === "Failed"
        ? "bg-destructive/10 text-destructive border-0"
        : "bg-primary/10 text-primary border-0"
  return (
    <Badge className={cn("text-xs font-semibold tracking-[-0.072px]", variant)}>
      {status}
    </Badge>
  )
}

/** Colored pill specifically for the EmbeddingJob status (Idle / Running / Failed). */
function EmbeddingStatusBadge({
  status,
}: {
  status: EmbeddingJob["status"]
}) {
  const cls =
    status === "Running"
      ? "bg-primary/15 text-primary border-0"
      : status === "Failed"
        ? "bg-destructive/10 text-destructive border-0"
        : "bg-muted text-muted-foreground border-0"
  return (
    <Badge className={cn("text-xs font-semibold", cls)}>{status}</Badge>
  )
}

/**
 * Inner pipelines page (wrapped by `<Suspense>` because of `useSearchParams`).
 *
 * Two tabs:
 * - "Data pipelines" — table-to-table ETL flows (uses `SAMPLE_PIPELINES`).
 * - "Vector jobs" — embedding job CRUD with a master/detail layout.
 *
 * The active tab is mirrored to the URL `?tab=vector` so the choice survives reloads.
 * Run history is shown via a right-side `Sheet` and the Agentic Builder lives in
 * a modal `Dialog`.
 */
function PipelinesPageInner() {
  const router = useRouter()
  const pathname = usePathname()
  const searchParams = useSearchParams()

  const tabFromUrl = searchParams.get("tab")
  const initialTab = tabFromUrl === TAB_VECTOR ? TAB_VECTOR : TAB_DATA
  const [tab, setTab] = useState(initialTab)

  // Keep tab state in sync with the URL (e.g. when the user navigates Back/Forward).
  // NOTE: triggers a `react-hooks/set-state-in-effect` lint error; intentionally kept
  // because router.replace is async and we want to avoid hydration flicker.
  useEffect(() => {
    const t = searchParams.get("tab")
    setTab(t === TAB_VECTOR ? TAB_VECTOR : TAB_DATA)
  }, [searchParams])

  /**
   * Updates the tab state AND mirrors it to the URL search params.
   *
   * Removes the `tab` param entirely for the default ("data") to keep URLs clean.
   * Uses `router.replace` (no history entry) and `scroll: false` to avoid jumps.
   */
  const setTabWithUrl = useCallback(
    (next: typeof TAB_DATA | typeof TAB_VECTOR) => {
      setTab(next)
      const params = new URLSearchParams(searchParams.toString())
      if (next === TAB_DATA) params.delete("tab")
      else params.set("tab", TAB_VECTOR)
      const q = params.toString()
      router.replace(q ? `${pathname}?${q}` : pathname, { scroll: false })
    },
    [pathname, router, searchParams]
  )

  const [runHistoryFor, setRunHistoryFor] = useState<PipelineItem | null>(null)
  const [showPipelineInfo, setShowPipelineInfo] = useState(false)
  const [pipelines, setPipelines] = useState<PipelineItem[]>(
    () => SAMPLE_PIPELINES.map((p) => ({ ...p }))
  )
  const [agenticBuilderOpen, setAgenticBuilderOpen] = useState(false)

  /**
   * Persists a pipeline to `sessionStorage` keyed by its id.
   *
   * The pipeline detail page (`/pipelines/[id]`) reads this back so a freshly
   * generated agentic pipeline can be opened immediately without a backend roundtrip.
   * Failures are silently swallowed (storage full / disabled / SSR).
   */
  const persistPipelineForDetail = useCallback((p: PipelineItem) => {
    try {
      sessionStorage.setItem(`rantai-pipeline-${p.id}`, JSON.stringify(p))
    } catch {
      /* storage full or unavailable */
    }
  }, [])

  /** Receives a generated pipeline from `AgenticPipelineBuilderDialog`, persists it, and prepends it to the list. */
  const handleAgenticPipelineGenerated = useCallback(
    (p: PipelineItem) => {
      persistPipelineForDetail(p)
      setPipelines((prev) => [p, ...prev])
    },
    [persistPipelineForDetail]
  )

  const [embeddingJobs, setEmbeddingJobs] = useState<EmbeddingJob[]>(
    () => INITIAL_EMBEDDING_JOBS
  )
  const [selectedEmbId, setSelectedEmbId] = useState(INITIAL_EMBEDDING_JOBS[0]!.id)
  const [embSaved, setEmbSaved] = useState(false)
  const [embRunning, setEmbRunning] = useState(false)

  const selectedJob = useMemo(
    () => embeddingJobs.find((j) => j.id === selectedEmbId),
    [embeddingJobs, selectedEmbId]
  )

  /** Patches the currently-selected embedding job in place via shallow merge. */
  const updateSelectedJob = useCallback(
    (patch: Partial<EmbeddingJob>) => {
      if (!selectedEmbId) return
      setEmbeddingJobs((jobs) =>
        jobs.map((j) => (j.id === selectedEmbId ? { ...j, ...patch } : j))
      )
    },
    [selectedEmbId]
  )

  /** Flashes a "Saved" badge for 2s. No real persistence — UI only. */
  const handleSaveEmbedding = useCallback(() => {
    setEmbSaved(true)
    window.setTimeout(() => setEmbSaved(false), 2000)
  }, [])

  /**
   * Simulates a vector-job run: flips the job to "Running" for ~2.2s, then back
   * to "Idle" with refreshed last-run metrics. Mock only.
   */
  const handleRunEmbedding = useCallback(() => {
    if (!selectedJob) return
    setEmbRunning(true)
    updateSelectedJob({ status: "Running" })
    window.setTimeout(() => {
      setEmbRunning(false)
      updateSelectedJob({
        status: "Idle",
        lastRun: new Date().toISOString().slice(0, 16).replace("T", " "),
        lastDuration: "2m 41s",
        vectorsUpserted: "12,889",
        indexSize: "856 MB",
      })
    }, 2200)
  }, [selectedJob, updateSelectedJob])

  /** Prepends a new blank vector job to the list and selects it for editing. */
  const addVectorJob = useCallback(() => {
    const id = newEmbeddingJobId()
    const job: EmbeddingJob = {
      id,
      name: "New vector job",
      enabled: true,
      model: "text-embedding-3-small",
      sourceColumn: "gold.your_table.text_column",
      targetIndex: "semantic.new_index",
      chunkTokens: "512",
      cron: "0 */6 * * *",
      notes: "",
      status: "Idle",
      lastRun: "—",
      lastDuration: "—",
      vectorsUpserted: "—",
      indexSize: "—",
    }
    setEmbeddingJobs((j) => [job, ...j])
    setSelectedEmbId(id)
  }, [])

  return (
    <div className="flex flex-col gap-4">
      <header className="flex flex-col gap-3 border-b border-border pb-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Pipelines
          </h1>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
            Orchestrate workloads in one place:{" "}
            <strong className="font-medium text-foreground">data pipelines</strong>{" "}
            move and transform tables across lake zones;{" "}
            <strong className="font-medium text-foreground">vector jobs</strong>{" "}
            embed text into indexes for semantic search.
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="default"
          className="h-10 shrink-0 gap-2 self-start border-primary/30 bg-primary/[0.06] text-primary hover:bg-primary/10 sm:self-auto"
          onClick={() => setAgenticBuilderOpen(true)}
        >
          <Sparkles className="size-4" aria-hidden />
          Agentic Builder
        </Button>
      </header>

      <Tabs
        value={tab}
        onValueChange={(v) =>
          setTabWithUrl(v === TAB_VECTOR ? TAB_VECTOR : TAB_DATA)
        }
        className="flex flex-col gap-4"
      >
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
          <div className="-mx-1 min-w-0 flex-1 overflow-x-auto px-1 pb-1 sm:max-w-xl">
            <TabsList className="h-auto min-h-9 w-max max-w-full gap-1 rounded-md bg-secondary p-1 sm:inline-flex sm:w-auto sm:flex-nowrap">
              <TabsTrigger
                value={TAB_DATA}
                className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
              >
                <Database className="size-4 shrink-0" />
                Data pipelines
              </TabsTrigger>
              <TabsTrigger
                value={TAB_VECTOR}
                className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
              >
                <Sparkles className="size-4 shrink-0" />
                Vector jobs
              </TabsTrigger>
            </TabsList>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-9 shrink-0 gap-2 self-start sm:self-auto"
            onClick={() => setShowPipelineInfo((v) => !v)}
            aria-expanded={showPipelineInfo}
            aria-controls="pipelines-comparison-info"
          >
            {showPipelineInfo ? (
              <>
                <EyeOff className="size-4" aria-hidden />
                Hide info
              </>
            ) : (
              <>
                <Eye className="size-4" aria-hidden />
                Show info
              </>
            )}
          </Button>
        </div>

        {/* Explainer — toggle with "Show info" / "Hide info" */}
        {showPipelineInfo && (
        <Card
          id="pipelines-comparison-info"
          className="border-primary/20 bg-primary/[0.04] shadow-none"
        >
          <CardContent className="flex flex-col gap-3 p-4 sm:flex-row sm:items-start sm:gap-6">
            <div className="flex min-w-0 flex-1 gap-3">
              <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary">
                <Database className="size-5" />
              </div>
              <div>
                <p className="text-sm font-semibold text-primary">
                  Data pipelines
                </p>
                <p className="mt-0.5 text-sm leading-relaxed text-muted-foreground">
                  ETL-style jobs: ingest, cleanse, and promote{" "}
                  <span className="font-mono text-xs">rows</span> between zones
                  (e.g. Raw → Bronze → Gold). Output is standard tables and
                  files.
                </p>
              </div>
            </div>
            <div className="hidden w-px shrink-0 bg-border sm:block" />
            <div className="flex min-w-0 flex-1 gap-3">
              <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary">
                <BrainCircuit className="size-5" />
              </div>
              <div>
                <p className="text-sm font-semibold text-primary">
                  Vector jobs
                </p>
                <p className="mt-0.5 text-sm leading-relaxed text-muted-foreground">
                  Embedding jobs: read text from a column, run an embedding
                  model, upsert <span className="font-mono text-xs">vectors</span>{" "}
                  into a semantic index. Powers Search &amp; similarity, not bulk
                  table moves.
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
        )}

        <TabsContent value={TAB_DATA} className="mt-0 space-y-4 outline-none">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-sm text-muted-foreground">
              Table-to-table pipelines and run history.
            </p>
            <div className="flex items-center gap-3">
              <Button
                variant="outline"
                size="default"
                type="button"
                className="h-10 gap-2 rounded-md border-border px-4 text-sm font-medium text-primary"
              >
                <RefreshCcw className="size-4" />
                Refresh
              </Button>
              <Button
                size="default"
                className="h-10 gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground"
                render={<Link href="/pipelines/create" />}
              >
                <Plus className="size-4" />
                Create data pipeline
              </Button>
            </div>
          </div>

          <section className="inline-flex w-fit max-w-full flex-wrap gap-2 self-start rounded-md bg-accent p-2">
            {SUMMARY_DATA.map((item) => (
              <Card
                key={item.label}
                className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm"
              >
                <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
                  <p className="text-sm font-medium leading-5 text-primary">
                    {item.label}
                  </p>
                </CardHeader>
                <CardContent className="px-3 pb-4 pt-1.5">
                  <p className="text-[24px] font-medium leading-none text-muted-foreground">
                    {item.value}
                  </p>
                </CardContent>
              </Card>
            ))}
          </section>

          <section className="grid gap-3 sm:grid-cols-2">
            {pipelines.map((pipeline) => (
              <Card
                key={pipeline.id}
                className="flex flex-col rounded-lg border border-border bg-card p-4 shadow-sm"
              >
                <div className="flex min-w-0 items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium text-primary">{pipeline.name}</p>
                    <p className="mt-0.5 line-clamp-2 text-sm text-muted-foreground">
                      {pipeline.description}
                    </p>
                  </div>
                  <StatusBadge status={pipeline.status} />
                </div>

                <p className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
                  <span className="font-medium text-foreground">{pipeline.flow.from}</span>
                  <ArrowRight className="size-3.5 shrink-0 opacity-70" aria-hidden />
                  <span className="font-medium text-foreground">{pipeline.flow.to}</span>
                </p>

                <p className="mt-2 text-xs text-muted-foreground">
                  <span className="inline-flex items-center gap-1">
                    <Clock className="size-3 shrink-0 opacity-80" aria-hidden />
                    {pipeline.lastRun}
                  </span>
                  <span className="mx-1.5 text-border" aria-hidden>
                    ·
                  </span>
                  {pipeline.schedule}
                  <span className="mx-1.5 text-border" aria-hidden>
                    ·
                  </span>
                  {pipeline.records}
                  {pipeline.successRate !== "—" ? (
                    <>
                      <span className="mx-1.5 text-border" aria-hidden>
                        ·
                      </span>
                      <span
                        className={
                          pipeline.status === "Failed"
                            ? "text-destructive"
                            : "text-[#008a2e]"
                        }
                      >
                        {pipeline.successRate}
                      </span>
                    </>
                  ) : null}
                </p>

                <div className="mt-4 flex flex-wrap gap-2 border-t border-border pt-3">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 gap-1.5 text-xs font-medium"
                    render={<Link href={`/pipelines/${pipeline.id}`} />}
                  >
                    View details
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 gap-1.5 text-xs font-medium"
                    onClick={() => setRunHistoryFor(pipeline)}
                  >
                    <History className="size-3.5" />
                    Run history
                  </Button>
                </div>
              </Card>
            ))}
          </section>
        </TabsContent>

        <TabsContent value={TAB_VECTOR} className="mt-0 space-y-4 outline-none">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-sm text-muted-foreground">
              Configure embedding models, sources, and schedules. Select a job to
              edit or run.
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="outline" type="button" className="h-10 gap-2">
                <RefreshCcw className="size-4" />
                Refresh
              </Button>
              <Button
                type="button"
                className="h-10 gap-2 bg-primary text-primary-foreground"
                onClick={addVectorJob}
              >
                <Plus className="size-4" />
                New vector job
              </Button>
            </div>
          </div>

          <section className="inline-flex w-fit max-w-full flex-wrap gap-2 self-start rounded-md bg-accent p-2">
            {SUMMARY_VECTOR.map((item) => (
              <Card
                key={item.label}
                className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm"
              >
                <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
                  <p className="text-sm font-medium leading-5 text-primary">
                    {item.label}
                  </p>
                </CardHeader>
                <CardContent className="px-3 pb-4 pt-1.5">
                  <p className="text-[24px] font-medium leading-none text-muted-foreground">
                    {item.value}
                  </p>
                </CardContent>
              </Card>
            ))}
          </section>

          <div className="grid gap-4 lg:grid-cols-[minmax(0,280px)_1fr]">
            <Card className="overflow-hidden rounded-lg border border-border shadow-sm">
              <CardHeader className="border-b border-border px-4 py-3">
                <h2 className="text-base font-medium text-primary">Jobs</h2>
                <p className="text-xs text-muted-foreground">
                  Click to load settings
                </p>
              </CardHeader>
              <CardContent className="max-h-[min(420px,55vh)] overflow-y-auto p-2">
                <ul className="flex flex-col gap-1">
                  {embeddingJobs.map((j) => (
                    <li key={j.id}>
                      <button
                        type="button"
                        onClick={() => setSelectedEmbId(j.id)}
                        className={cn(
                          "flex w-full flex-col gap-0.5 rounded-md border px-3 py-2.5 text-left text-sm transition-colors",
                          selectedEmbId === j.id
                            ? "border-primary bg-primary/10"
                            : "border-transparent hover:bg-muted/70"
                        )}
                      >
                        <span className="font-medium text-primary">{j.name}</span>
                        <span className="truncate font-mono text-[11px] text-muted-foreground">
                          {j.targetIndex}
                        </span>
                        <div className="mt-1 flex items-center gap-2">
                          <EmbeddingStatusBadge status={j.status} />
                          {!j.enabled && (
                            <Badge variant="outline" className="text-[10px]">
                              Disabled
                            </Badge>
                          )}
                        </div>
                      </button>
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>

            {selectedJob && (
              <div className="flex flex-col gap-4">
                <Card className="overflow-hidden rounded-lg border border-border shadow-sm">
                  <CardHeader className="flex flex-row flex-wrap items-center justify-between gap-3 border-b border-border px-4 py-3">
                    <div>
                      <h2 className="text-base font-medium text-primary">
                        Job settings
                      </h2>
                      <p className="text-xs text-muted-foreground">
                        {selectedJob.id}
                      </p>
                    </div>
                    <div className="flex items-center gap-3">
                      <div className="flex items-center gap-2">
                        <Label
                          htmlFor="emb-enabled"
                          className="text-xs text-muted-foreground"
                        >
                          Enabled
                        </Label>
                        <Switch
                          id="emb-enabled"
                          checked={selectedJob.enabled}
                          onCheckedChange={(v) =>
                            updateSelectedJob({ enabled: v })
                          }
                        />
                      </div>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="gap-2"
                        disabled={embRunning || !selectedJob.enabled}
                        onClick={handleRunEmbedding}
                      >
                        {embRunning ? (
                          <Loader2 className="size-4 animate-spin" />
                        ) : (
                          <Play className="size-4" />
                        )}
                        Run now
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        className="gap-2 bg-primary text-primary-foreground"
                        onClick={handleSaveEmbedding}
                      >
                        <Save className="size-4" />
                        Save
                      </Button>
                      {embSaved && (
                        <Badge className="border-0 bg-[#ecfdf2] text-[#008a2e]">
                          Saved
                        </Badge>
                      )}
                    </div>
                  </CardHeader>
                  <CardContent className="grid gap-4 p-4 sm:grid-cols-2">
                    <div className="sm:col-span-2 flex flex-col gap-1.5">
                      <Label htmlFor="ej-name">Display name</Label>
                      <Input
                        id="ej-name"
                        value={selectedJob.name}
                        onChange={(e) =>
                          updateSelectedJob({ name: e.target.value })
                        }
                        className="h-10"
                      />
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <Label>Embedding model</Label>
                      <Select
                        value={selectedJob.model}
                        onValueChange={(v) => {
                          if (v) updateSelectedJob({ model: v })
                        }}
                      >
                        <SelectTrigger className="h-10">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="text-embedding-3-small">
                            text-embedding-3-small
                          </SelectItem>
                          <SelectItem value="text-embedding-3-large">
                            text-embedding-3-large
                          </SelectItem>
                          <SelectItem value="rantai-embed-v1">rantai-embed-v1</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <Label htmlFor="ej-chunk">Chunk size (tokens)</Label>
                      <Input
                        id="ej-chunk"
                        value={selectedJob.chunkTokens}
                        onChange={(e) =>
                          updateSelectedJob({ chunkTokens: e.target.value })
                        }
                        className="h-10 font-mono text-sm"
                      />
                    </div>
                    <div className="sm:col-span-2 flex flex-col gap-1.5">
                      <Label htmlFor="ej-source">Source column</Label>
                      <Input
                        id="ej-source"
                        value={selectedJob.sourceColumn}
                        onChange={(e) =>
                          updateSelectedJob({ sourceColumn: e.target.value })
                        }
                        className="h-10 font-mono text-sm"
                      />
                    </div>
                    <div className="sm:col-span-2 flex flex-col gap-1.5">
                      <Label htmlFor="ej-index">Target vector index</Label>
                      <Input
                        id="ej-index"
                        value={selectedJob.targetIndex}
                        onChange={(e) =>
                          updateSelectedJob({ targetIndex: e.target.value })
                        }
                        className="h-10 font-mono text-sm"
                      />
                    </div>
                    <div className="sm:col-span-2 flex flex-col gap-1.5">
                      <Label htmlFor="ej-cron">Schedule (cron)</Label>
                      <div className="relative">
                        <Clock className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                        <Input
                          id="ej-cron"
                          value={selectedJob.cron}
                          onChange={(e) =>
                            updateSelectedJob({ cron: e.target.value })
                          }
                          className="h-10 pl-9 font-mono text-sm"
                        />
                      </div>
                    </div>
                    <div className="sm:col-span-2 flex flex-col gap-1.5">
                      <Label htmlFor="ej-notes">Notes</Label>
                      <Textarea
                        id="ej-notes"
                        rows={3}
                        value={selectedJob.notes}
                        onChange={(e) =>
                          updateSelectedJob({ notes: e.target.value })
                        }
                        placeholder="Normalization, filters, PII rules…"
                        className="min-h-[88px] text-sm"
                      />
                    </div>
                  </CardContent>
                </Card>

                <Card className="rounded-lg border border-border bg-muted/30 shadow-sm">
                  <CardHeader className="px-4 py-3">
                    <h2 className="text-base font-medium text-primary">
                      Last run
                    </h2>
                    <p className="text-xs text-muted-foreground">
                      Metrics from the latest completed execution
                    </p>
                  </CardHeader>
                  <CardContent className="grid gap-3 px-4 pb-4 sm:grid-cols-2">
                    <Metric label="Started" value={selectedJob.lastRun} />
                    <Metric label="Duration" value={selectedJob.lastDuration} />
                    <Metric
                      label="Vectors upserted"
                      value={selectedJob.vectorsUpserted}
                    />
                    <Metric label="Index size" value={selectedJob.indexSize} />
                  </CardContent>
                </Card>
              </div>
            )}
          </div>
        </TabsContent>
      </Tabs>

      <AgenticPipelineBuilderDialog
        open={agenticBuilderOpen}
        onOpenChange={setAgenticBuilderOpen}
        onPipelineGenerated={handleAgenticPipelineGenerated}
      />

      <Sheet
        open={runHistoryFor !== null}
        onOpenChange={(open) => {
          if (!open) setRunHistoryFor(null)
        }}
      >
        <SheetContent side="right" className="flex w-full flex-col sm:max-w-lg">
          <SheetHeader className="text-left">
            <SheetTitle>Run history</SheetTitle>
            <SheetDescription>
              {runHistoryFor ? (
                <>
                  Recent executions for{" "}
                  <span className="font-medium text-foreground">
                    {runHistoryFor.name}
                  </span>{" "}
                  ({runHistoryFor.flow.from} → {runHistoryFor.flow.to}).
                </>
              ) : null}
            </SheetDescription>
          </SheetHeader>
          <div className="mt-4 min-h-0 flex-1 overflow-auto rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-3">Run ID</TableHead>
                  <TableHead>Started</TableHead>
                  <TableHead>Duration</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="pr-3">Records</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {SAMPLE_RUN_HISTORY.map((run) => (
                  <TableRow key={run.id}>
                    <TableCell className="pl-3 font-mono text-xs text-primary">
                      {run.id}
                    </TableCell>
                    <TableCell className="whitespace-nowrap font-mono text-xs text-muted-foreground">
                      {run.started}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {run.duration}
                    </TableCell>
                    <TableCell>
                      <StatusBadge status={run.status} />
                    </TableCell>
                    <TableCell className="pr-3 text-muted-foreground">
                      {run.records}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
          <p className="mt-3 text-xs text-muted-foreground">
            Trigger type and orchestrator log links are available for each run.
          </p>
        </SheetContent>
      </Sheet>
    </div>
  )
}

/** Small label/value tile used in the "Last run" card. */
function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-border bg-background px-3 py-2">
      <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </p>
      <p className="mt-0.5 font-mono text-sm text-foreground">{value}</p>
    </div>
  )
}

/**
 * Default export for `/pipelines`.
 *
 * Wraps `PipelinesPageInner` in `<Suspense>` because that subtree calls
 * `useSearchParams`, which Next.js requires to be inside a Suspense boundary.
 */
export default function PipelinesPage() {
  return (
    <Suspense
      fallback={
        <div className="p-4 text-sm text-muted-foreground">Loading…</div>
      }
    >
      <PipelinesPageInner />
    </Suspense>
  )
}
