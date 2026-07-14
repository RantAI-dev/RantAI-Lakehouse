"use client"

import { useMemo } from "react"
import Link from "next/link"
import { useParams } from "next/navigation"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { getStaticPipelineById } from "@/lib/pipeline-fixtures"
import { cn } from "@/lib/utils"
import type { PipelineDetail, PipelineItem, PipelineRunResult } from "@/types/pipeline"

const STORAGE_PREFIX = "rantai-pipeline-"

const FALLBACK_DETAILS: Record<string, PipelineDetail> = {
  "1": {
    sourceConfig:
      "Raw zone: customer_revenue_stream (incremental on updated_at). Connector: internal lakehouse.",
    transformLogic:
      "Normalize currency, drop test accounts, join to region dimension, compute rolling 30d revenue.",
    destination: "Bronze.customer_revenue_curated (partitioned by ingest_date).",
    steps: [
      { title: "Extract", description: "Incremental pull from Raw customer events." },
      {
        title: "Transform",
        description: "Cast types, dedupe on customer_id + order_id, enrich region.",
      },
      { title: "Validate", description: "Row counts, null checks on amount and customer_id." },
      { title: "Load", description: "Merge into Bronze table with idempotent upsert." },
    ],
  },
  "2": {
    sourceConfig:
      "Raw zone: customer_revenue_stream (incremental on updated_at). Connector: internal lakehouse.",
    transformLogic:
      "Normalize currency, drop test accounts, join to region dimension, compute rolling 30d revenue.",
    destination: "Bronze.customer_revenue_curated (partitioned by ingest_date).",
    steps: [
      { title: "Extract", description: "Incremental pull from Raw customer events." },
      {
        title: "Transform",
        description: "Cast types, dedupe on customer_id + order_id, enrich region.",
      },
      { title: "Validate", description: "Row counts, null checks on amount and customer_id." },
      { title: "Load", description: "Merge into Bronze table with idempotent upsert." },
    ],
  },
  "3": {
    sourceConfig:
      "Bronze.sales_orders_fact and Silver.dim_store (batch window: last 7 days).",
    transformLogic:
      "Aggregate to store-day grain, compute sell-through and attach promo flags from reference table.",
    destination: "Silver.sales_store_daily rollup feeding Gold reporting marts.",
    steps: [
      { title: "Extract", description: "Batch read from Bronze fact and Silver dimensions." },
      {
        title: "Transform",
        description: "Group by store_id, day; compute KPIs and rolling averages.",
      },
      { title: "Validate", description: "Reconcile totals to source fact table subtotals." },
      { title: "Load", description: "Overwrite partition for the batch window in Silver." },
    ],
  },
  "4": {
    sourceConfig:
      "ERP inventory API snapshot + Raw.inventory_csv drops (hourly).",
    transformLogic:
      "Schema evolution handling, unit conversion to base UOM, flag negative on-hand as errors.",
    destination: "Bronze.inventory_snapshot — load blocked when validation fails.",
    steps: [
      { title: "Extract", description: "Pull latest snapshot from connector and staged files." },
      {
        title: "Transform",
        description: "Harmonize SKUs, convert packs to units, join location master.",
      },
      {
        title: "Validate",
        description: "Hard fail on negative quantity; quarantine bad rows to dead-letter.",
      },
      { title: "Load", description: "Transactional publish to Bronze (last run failed here)." },
    ],
  },
}

/** Returns the Tailwind classes for the colored status badge based on run status. */
function statusVariant(status: PipelineItem["status"]) {
  if (status === "Success")
    return "bg-[#ecfdf2] text-[#008a2e] border-0 hover:bg-[#ecfdf2]"
  if (status === "Failed")
    return "bg-destructive/10 text-destructive border-0"
  return "bg-primary/10 text-primary border-0"
}

/**
 * Reads a previously-persisted pipeline from `sessionStorage`.
 *
 * Used so newly generated pipelines (from the Agentic Builder on the list page)
 * can be opened on the detail page without a real backend. Returns `null` on SSR,
 * cache miss, or JSON parse failure.
 */
function readStoredPipeline(id: string): PipelineItem | null {
  if (typeof window === "undefined") return null
  try {
    const raw = sessionStorage.getItem(`${STORAGE_PREFIX}${id}`)
    if (!raw) return null
    return JSON.parse(raw) as PipelineItem
  } catch {
    return null
  }
}

/**
 * Card that renders the "Last successful run output" section, including the
 * run metadata (id / completedAt / duration / output table / rows written) and
 * a sample-rows preview table from `result.preview`.
 */
function PipelineSuccessOutputCard({
  pipeline,
  result,
}: {
  pipeline: PipelineItem
  result: PipelineRunResult
}) {
  return (
    <Card className="border-border shadow-sm">
      <CardHeader className="flex flex-col gap-2 border-b border-border pb-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
        <div className="min-w-0 space-y-1">
          <p className="text-base font-medium text-primary">Last successful run output</p>
          <p className="text-xs text-muted-foreground">
            Run <span className="font-mono text-foreground">{result.runId}</span>
            <span aria-hidden> · </span>
            {result.completedAt}
            <span aria-hidden> · </span>
            {result.duration}
          </p>
          {pipeline.status === "Running" ? (
            <p className="text-xs font-medium text-primary">Pipeline status: Running</p>
          ) : null}
          {result.previewNote ? (
            <p className="text-xs text-amber-800 dark:text-amber-400">{result.previewNote}</p>
          ) : null}
        </div>
        <Button
          variant="outline"
          size="sm"
          className="h-8 shrink-0 text-xs"
          render={<Link href="/data-catalog" />}
        >
          Open in Data catalog
        </Button>
      </CardHeader>
      <CardContent className="space-y-4 pt-4">
        <dl className="grid grid-cols-1 gap-2 sm:grid-cols-3">
          <div className="rounded-md border border-border bg-muted/30 px-3 py-2">
            <dt className="text-xs font-medium text-muted-foreground">Output table</dt>
            <dd className="mt-0.5 break-all font-mono text-xs text-foreground">{result.outputTable}</dd>
          </div>
          <div className="rounded-md border border-border bg-muted/30 px-3 py-2">
            <dt className="text-xs font-medium text-muted-foreground">Rows written</dt>
            <dd className="mt-0.5 text-sm font-medium text-foreground">{result.rowsWritten}</dd>
          </div>
          <div className="rounded-md border border-border bg-muted/30 px-3 py-2">
            <dt className="text-xs font-medium text-muted-foreground">Records (pipeline)</dt>
            <dd className="mt-0.5 text-sm font-medium text-foreground">{pipeline.records}</dd>
          </div>
        </dl>
        <div>
          <p className="mb-2 text-xs font-medium text-muted-foreground">
            Sample rows (preview only)
          </p>
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                {result.preview.columns.map((col) => (
                  <TableHead key={col} className="whitespace-nowrap font-mono text-xs">
                    {col}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {result.preview.rows.map((row, ri) => (
                <TableRow key={"preview-" + ri}>
                  {row.map((cell, ci) => (
                    <TableCell key={"preview-" + ri + "-" + ci} className="font-mono text-xs">
                      {cell}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  )
}

/**
 * Returns a `PipelineDetail` for a pipeline that doesn't ship one inline.
 *
 * 1. Tries the curated `FALLBACK_DETAILS` map (keyed by pipeline id).
 * 2. Otherwise generates a generic Extract/Transform/Validate/Load
 *    description from the pipeline's `flow` and `description` fields.
 */
function defaultDetailFromCard(p: PipelineItem): PipelineDetail {
  return (
    FALLBACK_DETAILS[p.id] ?? {
      sourceConfig: `${p.flow.from} → ${p.flow.to} flow. Connector details are available in the pipeline editor.`,
      transformLogic:
        "Transformation logic is defined in the visual builder and version-controlled YAML.",
      destination: `Destination tables in the ${p.flow.to} zone (configure in editor).`,
      steps: [
        { title: "Extract", description: `Read from ${p.flow.from} sources.` },
        { title: "Transform", description: p.description },
        { title: "Validate", description: "Quality checks before promotion." },
        { title: "Load", description: "Write to " + p.flow.to + " destination." },
      ],
    }
  )
}

/**
 * Pipeline detail page at `/pipelines/[id]`.
 *
 * Resolution order for pipeline data:
 * 1. `sessionStorage["rantai-pipeline-<id>"]` — set by the Agentic Builder so a
 *    just-generated draft can be opened immediately.
 * 2. `getStaticPipelineById(id)` — built-in fixture pipelines.
 *
 * If neither is present, a "Pipeline not found" empty state is shown.
 * Otherwise the page renders the run output (or a status-specific empty state)
 * plus the steps and source/destination/transform descriptions.
 */
export default function PipelineDetailPage() {
  const params = useParams()
  const id = typeof params.id === "string" ? params.id : ""

  const pipeline = useMemo(() => {
    const stored = readStoredPipeline(id)
    if (stored) return stored
    return getStaticPipelineById(id) ?? null
  }, [id])

  const detail: PipelineDetail | null = useMemo(() => {
    if (!pipeline) return null
    return pipeline.detail ?? defaultDetailFromCard(pipeline)
  }, [pipeline])

  if (!id) {
    return (
      <div className="flex flex-col gap-4 p-4">
        <header className="border-b border-border pb-2">
          <div className="flex flex-col gap-1">
            <Breadcrumb>
              <BreadcrumbList className="text-sm text-muted-foreground">
                <BreadcrumbItem>
                  <BreadcrumbLink href="/pipelines" render={<Link href="/pipelines" />}>
                    Pipelines
                  </BreadcrumbLink>
                </BreadcrumbItem>
                <BreadcrumbSeparator />
                <BreadcrumbItem>
                  <BreadcrumbPage className="font-normal text-primary">Pipeline</BreadcrumbPage>
                </BreadcrumbItem>
              </BreadcrumbList>
            </Breadcrumb>
            <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
              Missing pipeline id
            </h1>
          </div>
        </header>
        <p className="text-sm text-muted-foreground">This URL does not include a valid pipeline id.</p>
      </div>
    )
  }

  if (!pipeline || !detail) {
    return (
      <div className="flex flex-col gap-4 p-4">
        <header className="border-b border-border pb-2">
          <div className="flex flex-col gap-1">
            <Breadcrumb>
              <BreadcrumbList className="text-sm text-muted-foreground">
                <BreadcrumbItem>
                  <BreadcrumbLink href="/pipelines" render={<Link href="/pipelines" />}>
                    Pipelines
                  </BreadcrumbLink>
                </BreadcrumbItem>
                <BreadcrumbSeparator />
                <BreadcrumbItem>
                  <BreadcrumbPage className="font-normal text-primary">Not found</BreadcrumbPage>
                </BreadcrumbItem>
              </BreadcrumbList>
            </Breadcrumb>
            <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
              Pipeline not found
            </h1>
          </div>
        </header>
        <Card className="max-w-lg border-border">
          <CardContent className="pt-6">
            <p className="text-sm text-muted-foreground">
              Open this page from the pipeline list, or the draft may have expired if you cleared
              browser storage. Use{" "}
              <BreadcrumbLink
                href="/pipelines"
                className="font-medium text-primary underline-offset-4 hover:underline"
                render={<Link href="/pipelines" />}
              >
                Pipelines
              </BreadcrumbLink>{" "}
              in the breadcrumb to return.
            </p>
          </CardContent>
        </Card>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-4 p-4">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
        <div className="flex min-w-0 flex-col gap-1">
          <Breadcrumb>
            <BreadcrumbList className="text-sm text-muted-foreground">
              <BreadcrumbItem>
                <BreadcrumbLink href="/pipelines" render={<Link href="/pipelines" />}>
                  Pipelines
                </BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem>
                <BreadcrumbPage className="font-normal text-primary">{pipeline.name}</BreadcrumbPage>
              </BreadcrumbItem>
            </BreadcrumbList>
          </Breadcrumb>
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            {pipeline.name}
          </h1>
          <p className="max-w-2xl text-sm text-muted-foreground">{pipeline.description}</p>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">
              {pipeline.flow.from} → {pipeline.flow.to}
            </span>
            <span aria-hidden className="text-border">
              ·
            </span>
            <Badge
              className={cn("text-xs font-semibold tracking-[-0.072px]", statusVariant(pipeline.status))}
            >
              {pipeline.status}
            </Badge>
          </div>
        </div>
      </header>

      {pipeline.lastSuccessResult ? (
        <PipelineSuccessOutputCard pipeline={pipeline} result={pipeline.lastSuccessResult} />
      ) : pipeline.status === "Failed" ? (
        <Card className="border-border shadow-sm">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Run output</p>
            <p className="text-xs text-muted-foreground">
              The latest run did not complete successfully, so there is no new output to preview.
            </p>
          </CardHeader>
          <CardContent className="pt-2">
            <p className="text-sm text-muted-foreground">
              Fix validation or source errors, then run again. Historical successful outputs would
              appear here when available from the orchestrator.
            </p>
          </CardContent>
        </Card>
      ) : pipeline.status === "Success" ? (
        <Card className="border-border shadow-sm">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Run output</p>
            <p className="text-xs text-muted-foreground">No preview payload is attached to this pipeline yet.</p>
          </CardHeader>
        </Card>
      ) : (
        <Card className="border-border shadow-sm">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Run output</p>
            <p className="text-xs text-muted-foreground">
              Output preview appears after a successful run completes and the platform publishes a
              sample extract.
            </p>
          </CardHeader>
        </Card>
      )}

      <section className="grid gap-4 lg:grid-cols-2">
        <Card className="border-border shadow-sm lg:col-span-2">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Steps</p>
            <p className="text-xs text-muted-foreground">
              Ordered stages for this pipeline definition
            </p>
          </CardHeader>
          <CardContent className="grid gap-3 pt-4 sm:grid-cols-2">
            {detail.steps.map((s, i) => (
              <div
                key={"step-" + i + "-" + s.title}
                className="rounded-lg border border-border bg-muted/20 p-3"
              >
                <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Step {i + 1} - {s.title}
                </p>
                <p className="mt-1.5 text-sm leading-relaxed text-foreground">{s.description}</p>
              </div>
            ))}
          </CardContent>
        </Card>

        <Card className="border-border shadow-sm">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Source</p>
          </CardHeader>
          <CardContent className="pt-4">
            <p className="text-sm leading-relaxed text-muted-foreground">{detail.sourceConfig}</p>
          </CardContent>
        </Card>

        <Card className="border-border shadow-sm">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Destination</p>
          </CardHeader>
          <CardContent className="pt-4">
            <p className="text-sm leading-relaxed text-muted-foreground">{detail.destination}</p>
          </CardContent>
        </Card>

        <Card className="border-border shadow-sm lg:col-span-2">
          <CardHeader className="border-b border-border pb-3">
            <p className="text-base font-medium text-primary">Transformation logic</p>
          </CardHeader>
          <CardContent className="pt-4">
            <p className="whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground">
              {detail.transformLogic}
            </p>
          </CardContent>
        </Card>
      </section>

      <p className="text-xs text-muted-foreground">
        Edit scheduling, credentials, and fine-grained transforms in the full pipeline
        editor when connected to your environment.
      </p>
    </div>
  )
}
