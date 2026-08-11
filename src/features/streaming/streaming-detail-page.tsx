"use client"

import Link from "next/link"
import { useParams } from "next/navigation"
import { CodeBlock } from "@/components/patterns/code-block"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { MetadataList } from "@/components/patterns/metadata-list"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import { EntityHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useService } from "@/hooks/use-service"
import {
  formatBytes,
  formatLagSeconds,
  formatRate,
  formatRelativeTime,
} from "@/lib/format"
import { streamingService } from "@/services"
import type { StreamingJobDetail } from "@/services/contracts/streaming"

type Checkpoint = StreamingJobDetail["checkpoints"][number]

const checkpointColumns: ColumnDef<Checkpoint>[] = [
  { key: "at", header: "Checkpoint", render: (c) => formatRelativeTime(c.at) },
  { key: "size", header: "Size", render: (c) => formatBytes(c.sizeBytes) },
]

function MonoList({ items, ariaLabel }: { items: string[]; ariaLabel: string }) {
  return (
    <ul className="space-y-1.5" aria-label={ariaLabel}>
      {items.map((item) => (
        <li key={item} className="font-mono text-sm">
          {item}
        </li>
      ))}
    </ul>
  )
}

export function StreamingDetailPage() {
  const { jobId } = useParams<{ jobId: string }>()
  const state = useService((s) => streamingService.getJob(jobId, s), [jobId])
  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const j = state.data

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={<Link href="/streaming" className="hover:underline">Streaming Jobs</Link>}
        title={j.name}
        titleAccessory={<StatusBadge status={j.status} />}
      />
      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="definition">Definition</TabsTrigger>
          <TabsTrigger value="io">Sources & Sinks</TabsTrigger>
          <TabsTrigger value="triggers">Triggers</TabsTrigger>
          <TabsTrigger value="checkpoints">Checkpoints</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="mt-3 space-y-4">
          <MetricGrid>
            <MetricCard label="Lag" value={formatLagSeconds(j.lagSeconds)} />
            <MetricCard label="Throughput" value={formatRate(j.throughputPerSec)} />
            <MetricCard label="State size" value={formatBytes(j.stateSizeBytes)} />
            <MetricCard
              label="Watermark interval"
              value={`${j.watermarkIntervalSec}s`}
            />
          </MetricGrid>
          <SectionCard title="Details">
            <MetadataList
              items={[
                { label: "Owner", value: j.owner },
                { label: "Last barrier", value: formatRelativeTime(j.lastBarrierAt) },
                { label: "Sources", value: <span className="font-mono text-xs">{j.sources.join(", ")}</span> },
                { label: "Sinks", value: <span className="font-mono text-xs">{j.sinks.join(", ")}</span> },
              ]}
            />
          </SectionCard>
        </TabsContent>
        <TabsContent value="definition" className="mt-3">
          <CodeBlock>{j.definitionSql}</CodeBlock>
        </TabsContent>
        <TabsContent value="io" className="mt-3">
          <div className="grid gap-3 lg:grid-cols-2">
            <SectionCard title="Sources">
              <MonoList items={j.sources} ariaLabel="Sources" />
            </SectionCard>
            <SectionCard title="Sinks">
              <MonoList items={j.sinks} ariaLabel="Sinks" />
            </SectionCard>
          </div>
        </TabsContent>
        <TabsContent value="triggers" className="mt-3">
          {j.triggers.length === 0 ? (
            <EmptyState
              title="No triggers"
              description="This job has no agent triggers configured."
            />
          ) : (
            <SectionCard title="Triggers">
              <ul className="space-y-2 text-sm">
                {j.triggers.map((t) => (
                  <li key={t.id} className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-xs">{t.condition}</span>
                    <span className="text-muted-foreground" aria-hidden>→</span>
                    {t.target.startsWith("/") ? (
                      <Link href={t.target} className="text-primary hover:underline">
                        {t.target}
                      </Link>
                    ) : (
                      <span>{t.target}</span>
                    )}
                  </li>
                ))}
              </ul>
            </SectionCard>
          )}
        </TabsContent>
        <TabsContent value="checkpoints" className="mt-3">
          {j.checkpoints.length === 0 ? (
            <EmptyState
              title="No checkpoints"
              description="Checkpoints appear here as the job creates barriers."
            />
          ) : (
            <DataTable
              columns={checkpointColumns}
              rows={j.checkpoints}
              rowKey={(c) => c.id}
            />
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
