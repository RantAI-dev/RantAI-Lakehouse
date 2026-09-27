"use client"

import * as React from "react"
import Link from "next/link"
import {
  ArrowDownIcon,
  ChevronDownIcon,
  Code2Icon,
  CopyXIcon,
  DatabaseIcon,
  FilterIcon,
  ListChecksIcon,
  PencilLineIcon,
  ReplaceIcon,
} from "lucide-react"
import { CodeView } from "@/components/patterns/code-view"
import { FlowCanvas } from "@/components/patterns/flow-canvas"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import type { EntityStatus } from "@/lib/status"
import { describeTransform, type TransformDraft } from "@/lib/transform-draft"
import { cn } from "@/lib/utils"
import { pipelineService } from "@/services"
import type { PipelineDetail, PipelineOpNode } from "@/services/contracts/pipelines"
import { topoSortOps } from "./topo-sort-ops"

const VERB_ICON: Record<TransformDraft["verb"], React.ComponentType<{ className?: string }>> = {
  dedupe: CopyXIcon,
  filter: FilterIcon,
  rename: PencilLineIcon,
  cast: ReplaceIcon,
  select: ListChecksIcon,
}

function TableNode({
  role,
  zone,
  table,
  assetId,
}: {
  role: "Source" | "Target"
  zone: string
  table: string
  assetId?: string
}) {
  const body = (
    <>
      <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary">
        <DatabaseIcon className="size-4" aria-hidden />
      </span>
      <span className="min-w-0">
        <span className="block font-mono text-[10px] uppercase tracking-[0.14em] text-muted-foreground">{role}</span>
        <span className="block truncate font-mono text-sm font-medium">
          <span className="text-muted-foreground">{zone}.</span>
          {table}
        </span>
      </span>
    </>
  )
  const cls = "flex items-center gap-3 rounded-xl border border-border bg-card px-4 py-3 shadow-xs"
  return assetId ? (
    <Link href={`/data/assets/${assetId}`} className={cn(cls, "hover:border-primary/40")}>
      {body}
    </Link>
  ) : (
    <div className={cls}>{body}</div>
  )
}

/**
 * An authored pipeline's stored definition drawn as the path its rows
 * take: source table, each transform in order, target table. The
 * transforms are the exact strings `transform_grammar::parse_transform`
 * accepted when the pipeline was saved; each is shown with its verb and
 * the raw string, so what you read is what runs.
 */
export function AuthoredFlow({ pipeline }: { pipeline: PipelineDetail }) {
  const def = pipeline.definition
  if (!def) {
    return <EmptyState title="No definition stored" description="This authored pipeline has no stored definition to show." />
  }
  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,26rem)_1fr]">
      <ol className="relative flex flex-col gap-2">
        <li>
          <TableNode role="Source" zone={def.sourceZone} table={def.sourceTable} assetId={pipeline.sourceAssetId} />
        </li>
        {def.transforms.length === 0 ? (
          <li className="flex items-center gap-2 pl-6 text-xs text-muted-foreground">
            <ArrowDownIcon className="size-3.5" aria-hidden />
            copied as is, no transforms
          </li>
        ) : (
          def.transforms.map((raw, i) => {
            const t = describeTransform(raw)
            const Icon = t ? VERB_ICON[t.verb] : Code2Icon
            return (
              <li key={`${raw}-${i}`} className="relative pl-6">
                <span aria-hidden className="absolute left-[1.05rem] top-0 h-full w-px bg-border" />
                <div className="relative flex items-start gap-3 rounded-lg border border-dashed border-border bg-muted/30 px-3 py-2">
                  <span className="mt-0.5 font-mono text-[10px] tabular-nums text-muted-foreground">{String(i + 1).padStart(2, "0")}</span>
                  <Icon className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden />
                  <span className="min-w-0">
                    <span className="block text-sm font-medium capitalize">{t ? t.verb : "transform"}</span>
                    {t ? <span className="block text-xs text-muted-foreground">{t.detail}</span> : null}
                    <code className="mt-1 block truncate text-[11px] text-muted-foreground">{raw}</code>
                  </span>
                </div>
              </li>
            )
          })
        )}
        <li>
          <TableNode role="Target" zone={def.targetZone} table={def.targetTable} assetId={pipeline.targetAssetId} />
        </li>
      </ol>
      <dl className="grid content-start gap-x-6 gap-y-3 self-start rounded-xl border border-border bg-card p-4 text-sm sm:grid-cols-2">
        <div>
          <dt className="text-xs text-muted-foreground">Load</dt>
          <dd className="font-medium">
            {def.incrementalColumn ? (
              <>
                Incremental on <span className="font-mono">{def.incrementalColumn}</span>
              </>
            ) : (
              "Full reload each run"
            )}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Change capture (FBIC)</dt>
          <dd className="font-medium">{def.fbicEnabled ? "On" : "Off"}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Connector</dt>
          <dd className="font-mono text-xs">
            {def.connectorId ? (
              <Link href="/connectors" className="text-primary hover:underline">
                {def.connectorId}
              </Link>
            ) : (
              "—"
            )}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Transforms</dt>
          <dd className="font-medium tabular-nums">{def.transforms.length}</dd>
        </div>
      </dl>
    </div>
  )
}

/**
 * One op's read-only source. A 404 (unknown op) or 409 (the build's
 * commit does not match, `pipeline_source.rs`'s `check_commit`) shows the
 * server's own message. The request is keyed by `sourceRef`, what the
 * route matches on, never the op name.
 */
function OpSource({ pipelineId, sourceRef }: { pipelineId: string; sourceRef: string }) {
  const source = useService((s) => pipelineService.getPipelineSource(pipelineId, sourceRef, s), [pipelineId, sourceRef])
  if (source.status === "loading") return <LoadingSkeleton rows={5} />
  if (source.status === "error") return <ErrorState error={source.error} onRetry={source.reload} />
  return (
    <div className="flex flex-col gap-1.5">
      <p className="font-mono text-[11px] text-muted-foreground">commit {source.data.commit.slice(0, 12)}</p>
      <CodeView text={source.data.text} className="max-h-[28rem] overflow-auto" />
    </div>
  )
}

function OpCard({ pipelineId, op, upstream }: { pipelineId: string; op: PipelineOpNode; upstream: string[] }) {
  const [open, setOpen] = React.useState(false)
  const [showSource, setShowSource] = React.useState(false)
  const doc = op.description?.trim() ?? ""
  const [lead, ...rest] = doc.split(/\n\s*\n/)
  return (
    <article className="rounded-xl border border-border bg-card p-4">
      <header className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <h4 className="font-mono text-sm font-semibold">{op.name}</h4>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {upstream.length > 0 ? `after ${upstream.join(", ")}` : "entry step"}
            {op.sourceRef ? (
              <>
                {" · "}
                <span className="font-mono">{op.sourceRef}</span>
              </>
            ) : null}
          </p>
        </div>
        {op.sourceRef ? (
          <Button size="xs" variant="outline" onClick={() => setShowSource((v) => !v)} aria-expanded={showSource}>
            <Code2Icon data-icon="inline-start" />
            {showSource ? "Hide source" : "View source"}
          </Button>
        ) : null}
      </header>
      {doc ? (
        <div className="mt-3 text-sm leading-6 text-foreground/90">
          <p className="whitespace-pre-line text-sm">{lead}</p>
          {rest.length > 0 ? (
            <>
              {open ? <p className="mt-2 whitespace-pre-line text-sm text-muted-foreground">{rest.join("\n\n")}</p> : null}
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                className="mt-1 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
              >
                <ChevronDownIcon className={cn("size-3.5 transition-transform", open && "rotate-180")} aria-hidden />
                {open ? "Less" : "Read the full description"}
              </button>
            </>
          ) : null}
        </div>
      ) : (
        <p className="mt-3 text-xs text-muted-foreground">The op declares no description.</p>
      )}
      {op.sql ? (
        <pre className="mt-3 overflow-x-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs">{op.sql}</pre>
      ) : null}
      {showSource && op.sourceRef ? (
        <div className="mt-3">
          <OpSource pipelineId={pipelineId} sourceRef={op.sourceRef} />
        </div>
      ) : null}
    </article>
  )
}

/**
 * A Dagster job's op graph, coloured by the selected run's step statuses,
 * then each op with its own description and source. The description is
 * the op's docstring, published by the code location, which says what the
 * step does in the author's words.
 */
export function DagsterDefinition({
  pipeline,
  stepStatus,
  runLabel,
}: {
  pipeline: PipelineDetail
  stepStatus: (op: string) => EntityStatus | undefined
  runLabel: string | null
}) {
  const graph = pipeline.graph
  if (!graph || graph.ops.length === 0) {
    return (
      <EmptyState
        title="No op graph for this job"
        description="The orchestrator could not resolve this job's graph for the running build."
      />
    )
  }
  const sorted = topoSortOps(graph.ops, graph.edges)
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <p className="text-xs text-muted-foreground">
          {sorted.length} {sorted.length === 1 ? "op" : "ops"}
          {runLabel ? ` · statuses from ${runLabel}` : ""}
        </p>
        <FlowCanvas
          nodes={sorted.map(({ node, upstream }) => ({
            id: node.name,
            label: node.name,
            sublabel: upstream.length > 0 ? `after: ${upstream.join(", ")}` : undefined,
            status: stepStatus(node.name),
          }))}
        />
      </div>
      <div className="grid gap-3">
        {sorted.map(({ node, upstream }) => (
          <OpCard key={node.name} pipelineId={pipeline.id} op={node} upstream={upstream} />
        ))}
      </div>
    </div>
  )
}
