"use client"

import * as React from "react"
import Link from "next/link"
import { ChevronDownIcon, FileCode2Icon, InfoIcon } from "lucide-react"
import { CodeView } from "@/components/patterns/code-view"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { describeTransform } from "@/lib/transform-draft"
import { cn } from "@/lib/utils"
import { pipelineService } from "@/services"
import type { PipelineDetail, PipelineOpNode, PipelineRunStep } from "@/services/contracts/pipelines"
import { authoredFlow, dagsterFlow } from "./pipeline-flow-layout"
import { PipelineFlowchart } from "./pipeline-flowchart"

function PanelTitle({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="font-mono text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">{children}</h4>
  )
}

function PhraseList({ title, phrases }: { title: string; phrases: string[] }) {
  return (
    <div>
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      {phrases.length === 0 ? (
        <p className="mt-1 text-xs text-muted-foreground">Nothing declared.</p>
      ) : (
        <ul className="mt-1 flex flex-col gap-1">
          {phrases.map((p) => (
            <li key={p} className="rounded-md bg-muted/50 px-2 py-1 font-mono text-[11px] leading-4">
              {p}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/** A docstring's first paragraph, with the rest behind a toggle. */
function Docstring({ text }: { text: string | null }) {
  const [open, setOpen] = React.useState(false)
  const doc = text?.trim() ?? ""
  if (!doc) return <p className="text-xs text-muted-foreground">The op declares no description.</p>
  const [lead, ...rest] = doc.split(/\n\s*\n/)
  return (
    <div className="text-sm leading-6 text-foreground/90">
      <p className="whitespace-pre-line">{lead}</p>
      {rest.length > 0 ? (
        <>
          {open ? <p className="mt-2 whitespace-pre-line text-muted-foreground">{rest.join("\n\n")}</p> : null}
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
  )
}

/**
 * One op's source, read from the code baked into the API image. A 404
 * (unknown op) or 409 (the op's image and the API's were built from
 * different commits, or with no commit recorded: `check_commit`) shows the
 * server's own message, because serving "the current file" then could
 * show code that is not what ran. The request is keyed by `sourceRef`,
 * which is what the route matches on.
 */
function OpSource({ pipelineId, op }: { pipelineId: string; op: PipelineOpNode }) {
  const sourceRef = op.sourceRef
  const source = useService(
    (s) => (sourceRef ? pipelineService.getPipelineSource(pipelineId, sourceRef, s) : Promise.resolve(null)),
    [pipelineId, sourceRef]
  )
  if (!sourceRef) {
    return (
      <p className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">
        This op declares no source location, so there is no code to show.
      </p>
    )
  }
  if (source.status === "loading") return <LoadingSkeleton rows={8} />
  if (source.status === "error") {
    return (
      <div className="flex flex-col gap-2">
        <ErrorState error={source.error} onRetry={source.reload} />
        <p className="text-xs text-muted-foreground">
          Source is served only when the API and the Dagster images were built from the same commit (
          <code className="font-mono">GIT_SHA</code>); otherwise the file in the API image may not be the code that ran.
          This op reports commit <code className="font-mono">{op.commit ?? "none"}</code>.
        </p>
      </div>
    )
  }
  if (!source.data) return null
  return (
    <div className="flex flex-col gap-1.5">
      <p className="flex flex-wrap items-center gap-x-3 font-mono text-[11px] text-muted-foreground">
        <span>{source.data.sourceRef}</span>
        <span>commit {source.data.commit.slice(0, 12)}</span>
      </p>
      <CodeView text={source.data.text} language={source.data.language} className="max-h-[34rem] overflow-auto" />
    </div>
  )
}

/**
 * A Dagster job as a flowchart: what its ops declare they read, the ops in
 * dependency order coloured by the selected run, what they declare they
 * write. Clicking an op opens its description and its source code below.
 */
export function DagsterDefinition({
  pipeline,
  steps,
  runLabel,
}: {
  pipeline: PipelineDetail
  steps: PipelineRunStep[] | null
  runLabel: string | null
}) {
  const graph = pipeline.graph
  const ops = React.useMemo(() => graph?.ops ?? [], [graph])
  const [selected, setSelected] = React.useState<string | null>(null)
  const flow = React.useMemo(
    () =>
      dagsterFlow(ops, graph?.edges ?? [], (name) => steps?.find((s) => s.stepKey === name)?.status),
    [ops, graph, steps]
  )
  const durations = React.useMemo(() => {
    const out: Record<string, number | null> = {}
    for (const s of steps ?? []) {
      out[`op:${s.stepKey}`] = s.startMs !== null && s.endMs !== null ? s.endMs - s.startMs : null
    }
    return out
  }, [steps])

  if (!graph || ops.length === 0) {
    return (
      <EmptyState
        title="No op graph for this job"
        description="The orchestrator could not resolve this job's graph for the running build."
      />
    )
  }
  const selectedId = selected ?? `op:${ops[0].name}`
  const op = ops.find((o) => `op:${o.name}` === selectedId)
  const dataNode = flow.nodes.find((n) => n.id === selectedId && n.kind !== "op")

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
        <span>
          {ops.length} {ops.length === 1 ? "op" : "ops"}
          {runLabel ? ` · statuses from ${runLabel}` : " · no run selected"}
        </span>
        <span className="inline-flex items-center gap-1.5">
          <InfoIcon className="size-3.5" aria-hidden />
          Reads and writes are declared in each op&apos;s code, not observed from a run
        </span>
      </div>
      <PipelineFlowchart
        nodes={flow.nodes}
        edges={flow.edges}
        selectedId={selectedId}
        onSelect={(n) => setSelected(n.id)}
        durations={durations}
      />

      {op ? (
        <div className="grid gap-4 lg:grid-cols-[minmax(0,22rem)_minmax(0,1fr)]">
          <section className="flex flex-col gap-4 self-start rounded-xl border border-border bg-card p-4">
            <div>
              <PanelTitle>Op</PanelTitle>
              <p className="mt-1 font-mono text-sm font-semibold">{op.name}</p>
            </div>
            <Docstring text={op.description} />
            <PhraseList title="Reads" phrases={op.reads ?? []} />
            <PhraseList title="Writes" phrases={op.writes ?? []} />
            {op.sql ? (
              <div>
                <p className="text-xs font-medium text-muted-foreground">SQL</p>
                <CodeView text={op.sql} language="sql" className="mt-1" />
              </div>
            ) : null}
          </section>
          <section className="flex min-w-0 flex-col gap-2 rounded-xl border border-border bg-card p-4">
            <div className="flex items-center gap-2">
              <FileCode2Icon className="size-4 text-primary" aria-hidden />
              <PanelTitle>Source code</PanelTitle>
            </div>
            <OpSource pipelineId={pipeline.id} op={op} />
          </section>
        </div>
      ) : dataNode ? (
        <section className="rounded-xl border border-border bg-card p-4">
          <PanelTitle>{dataNode.kind === "source" ? "Read by" : "Written by"}</PanelTitle>
          <p className="mt-1 font-mono text-sm">{dataNode.label}</p>
          <p className="mt-2 text-xs text-muted-foreground">
            {flow.edges
              .filter((e) => e.from === dataNode.id || e.to === dataNode.id)
              .map((e) => (e.from === dataNode.id ? e.to : e.from).replace(/^op:/, ""))
              .join(", ")}
            , as declared in the op&apos;s code. Select the op to see that code.
          </p>
        </section>
      ) : null}
    </div>
  )
}

/**
 * An authored pipeline as a flowchart: source table, each transform in
 * order, target table. Below it, the selected step and the stored
 * definition itself: the exact record the engine reads, since an authored
 * pipeline has no code of its own. Its transforms are the strings
 * `transform_grammar::parse_transform` accepted when it was saved.
 */
export function AuthoredFlow({ pipeline }: { pipeline: PipelineDetail }) {
  const def = pipeline.definition
  const [selected, setSelected] = React.useState<string>("source")
  const flow = React.useMemo(() => (def ? authoredFlow(def, describeTransform) : null), [def])
  if (!def || !flow) {
    return <EmptyState title="No definition stored" description="This authored pipeline has no stored definition to show." />
  }
  const node = flow.nodes.find((n) => n.id === selected) ?? flow.nodes[0]
  const transformIndex = node.kind === "transform" && node.ref !== undefined ? Number(node.ref) : null
  const assetId = node.id === "source" ? pipeline.sourceAssetId : node.id === "target" ? pipeline.targetAssetId : undefined

  return (
    <div className="flex flex-col gap-4">
      <PipelineFlowchart
        nodes={flow.nodes}
        edges={flow.edges}
        selectedId={node.id}
        onSelect={(n) => setSelected(n.id)}
        dataCaption={{ source: "source table", sink: "target table" }}
      />
      <div className="grid gap-4 lg:grid-cols-[minmax(0,22rem)_minmax(0,1fr)]">
        <section className="flex flex-col gap-3 self-start rounded-xl border border-border bg-card p-4 text-sm">
          {transformIndex !== null ? (
            <>
              <PanelTitle>Transform {transformIndex + 1} of {def.transforms.length}</PanelTitle>
              <p className="font-medium capitalize">{node.label}</p>
              <p className="text-muted-foreground">{node.sublabel}</p>
              <code className="rounded-md bg-muted/50 px-2 py-1 font-mono text-xs">{def.transforms[transformIndex]}</code>
            </>
          ) : (
            <>
              <PanelTitle>{node.id === "source" ? "Source table" : "Target table"}</PanelTitle>
              <p className="font-mono">{node.label}</p>
              {assetId ? (
                <Link href={`/data/assets/${assetId}`} className="text-xs text-primary hover:underline">
                  Open in the catalog
                </Link>
              ) : null}
            </>
          )}
          <dl className="mt-1 grid grid-cols-2 gap-x-4 gap-y-2 border-t border-border pt-3 text-xs">
            <dt className="text-muted-foreground">Load</dt>
            <dd>{def.incrementalColumn ? `Incremental on ${def.incrementalColumn}` : "Full reload each run"}</dd>
            <dt className="text-muted-foreground">Change capture (FBIC)</dt>
            <dd>{def.fbicEnabled ? "On" : "Off"}</dd>
            <dt className="text-muted-foreground">Connector</dt>
            <dd className="font-mono">{def.connectorId ?? "—"}</dd>
          </dl>
        </section>
        <section className="flex min-w-0 flex-col gap-2 rounded-xl border border-border bg-card p-4">
          <div className="flex items-center gap-2">
            <FileCode2Icon className="size-4 text-primary" aria-hidden />
            <PanelTitle>Stored definition</PanelTitle>
          </div>
          <p className="text-xs text-muted-foreground">
            The record the engine reads. An authored pipeline has no code of its own: the generic job in{" "}
            <code className="font-mono">authored_factory.py</code> reads this and builds its SQL from the transforms.
          </p>
          <CodeView text={JSON.stringify(def, null, 2)} language="plain" />
        </section>
      </div>
    </div>
  )
}
