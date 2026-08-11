"use client"

import Link from "next/link"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { CheckBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { formatCompactNumber, formatRelativeTime } from "@/lib/format"
import type { AssetDetail } from "@/services/contracts/assets"

function QuietEmpty({ title }: { title: string }) {
  return <EmptyState title={title} className="py-4" />
}

function dependentHref(id: string, kind: string) {
  if (kind.toLowerCase().includes("pipeline")) return `/pipelines/${id}`
  if (kind.toLowerCase().includes("agent")) return `/agents/employees/${id}`
  if (kind.toLowerCase().includes("dashboard") || kind.toLowerCase().includes("query")) {
    return `/query-studio?saved=${id}`
  }
  return `/data/assets/${id}`
}

/** Tab strip for the asset detail page: schema, sample, quality, and metadata. */
export function AssetDetailTabs({ asset: a }: { asset: AssetDetail }) {
  const sampleColumns = Object.keys(a.sample[0] ?? {})

  return (
    <Tabs defaultValue="schema" className="gap-1.5">
      <TabsList>
        <TabsTrigger value="schema">Schema</TabsTrigger>
        <TabsTrigger value="sample">Sample</TabsTrigger>
        <TabsTrigger value="quality">Quality</TabsTrigger>
        <TabsTrigger value="policies">Policies</TabsTrigger>
        <TabsTrigger value="lineage">Lineage</TabsTrigger>
        <TabsTrigger value="dependents">Dependents</TabsTrigger>
        <TabsTrigger value="history">History</TabsTrigger>
        <TabsTrigger value="snapshots">Snapshots</TabsTrigger>
        <TabsTrigger value="usage">Usage</TabsTrigger>
      </TabsList>

      <TabsContent value="schema" className="mt-2 flex flex-col gap-2">
        <SectionCard size="sm" title="Columns">
          {a.schema.length === 0 ? (
            <QuietEmpty title="No columns registered" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.schema.map((c) => (
                <li key={c.name} className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 py-1.5">
                  <span className="font-mono font-medium">{c.name}</span>
                  <span className="text-muted-foreground">{c.dataType}</span>
                  {c.masked ? (
                    <span className="text-xs text-amber-600 dark:text-amber-400">
                      masked
                    </span>
                  ) : null}
                  {c.description ? (
                    <span className="text-muted-foreground">— {c.description}</span>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
        <SectionCard
          size="sm"
          title="Schema versions"
          description="Registered schema changes, most recent first."
        >
          {a.schemaVersions.length === 0 ? (
            <QuietEmpty title="No schema versions recorded" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.schemaVersions.map((v) => (
                <li key={v.version} className="flex flex-wrap items-baseline gap-2 py-1.5">
                  <span className="font-mono text-xs text-muted-foreground">
                    v{v.version}
                  </span>
                  <span>{v.change}</span>
                  <span className="ml-auto text-xs text-muted-foreground">
                    {formatRelativeTime(v.at)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="sample" className="mt-2">
        <SectionCard
          size="sm"
          title="Sample rows"
          description="Masked values applied where policy requires."
        >
          {a.sample.length === 0 ? (
            <QuietEmpty title="No sample rows available" />
          ) : (
            <div className="overflow-hidden rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow className="hover:bg-transparent">
                    {sampleColumns.map((col) => (
                      <TableHead key={col} className="font-mono text-xs font-medium">
                        {col}
                      </TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {a.sample.map((row, i) => (
                    <TableRow key={i}>
                      {sampleColumns.map((col) => (
                        <TableCell key={col} className="py-1.5 font-mono text-xs">
                          {row[col] ?? "—"}
                        </TableCell>
                      ))}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="quality" className="mt-2">
        <SectionCard
          size="sm"
          title="Quality checks"
          description="Rules evaluated against this asset."
          action={
            <Button size="sm" variant="outline" render={<Link href="/governance/data-quality" />}>
              Open data quality
            </Button>
          }
        >
          {a.qualityChecks.length === 0 ? (
            <QuietEmpty title="No quality checks configured" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.qualityChecks.map((q) => (
                <li key={q.id} className="flex flex-wrap items-center gap-2 py-1.5">
                  <span>
                    {q.name} · <span className="text-muted-foreground">{q.dimension}</span>
                  </span>
                  <span className="ml-auto flex items-center gap-2">
                    <CheckBadge status={q.status} />
                    <span className="text-xs text-muted-foreground">
                      {formatRelativeTime(q.lastRun)}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="policies" className="mt-2">
        <SectionCard
          size="sm"
          title="Policies"
          description="Access, masking, and residency rules applied to this asset."
          action={
            <Button size="sm" variant="outline" render={<Link href="/governance/policies" />}>
              Open policies
            </Button>
          }
        >
          {a.policySummary.length === 0 ? (
            <QuietEmpty title="No policies applied" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.policySummary.map((p) => (
                <li key={p.id} className="py-1.5">
                  <Link
                    href={`/governance/policies?id=${p.id}`}
                    className="font-medium text-primary hover:underline"
                  >
                    {p.name}
                  </Link>
                  <p className="text-xs text-muted-foreground">{p.effect}</p>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="lineage" className="mt-2">
        <div className="mb-2 flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            render={<Link href={`/lineage?focus=${a.id}`} />}
          >
            Open lineage graph
          </Button>
          <Button size="sm" variant="ghost" render={<Link href="/storage" />}>
            Storage lifecycle
          </Button>
          <Button size="sm" variant="ghost" render={<Link href="/query-studio" />}>
            Query this asset
          </Button>
        </div>
        <div className="grid gap-2 lg:grid-cols-2">
          <SectionCard size="sm" title="Upstream">
            {a.upstream.length === 0 ? (
              <QuietEmpty title="No upstream assets" />
            ) : (
              <ul className="space-y-1 text-sm">
                {a.upstream.map((u) => (
                  <li key={u.id}>
                    <Link
                      href={`/data/assets/${u.id}`}
                      className="text-primary hover:underline"
                    >
                      {u.name}
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </SectionCard>
          <SectionCard size="sm" title="Downstream">
            {a.downstream.length === 0 ? (
              <QuietEmpty title="No downstream assets" />
            ) : (
              <ul className="space-y-1 text-sm">
                {a.downstream.map((u) => (
                  <li key={u.id}>
                    <Link
                      href={`/data/assets/${u.id}`}
                      className="text-primary hover:underline"
                    >
                      {u.name}
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </SectionCard>
        </div>
      </TabsContent>

      <TabsContent value="dependents" className="mt-2">
        <SectionCard
          size="sm"
          title="Dependents"
          description="Pipelines, dashboards, and agents consuming this asset."
        >
          {a.dependents.length === 0 ? (
            <QuietEmpty title="No dependents registered" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.dependents.map((d) => (
                <li key={d.id} className="flex items-center gap-2 py-1.5">
                  <Link
                    href={dependentHref(d.id, d.kind)}
                    className="font-mono text-primary hover:underline"
                  >
                    {d.name}
                  </Link>
                  <Pill tone="neutral" className="ml-auto">
                    {d.kind}
                  </Pill>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="history" className="mt-2">
        <SectionCard size="sm" title="Change history">
          {a.changeHistory.length === 0 ? (
            <QuietEmpty title="No changes recorded" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.changeHistory.map((c) => (
                <li key={c.id} className="flex flex-wrap items-baseline gap-2 py-1.5">
                  <span className="font-medium">{c.actor}</span>
                  <span className="text-muted-foreground">{c.summary}</span>
                  <span className="ml-auto text-xs text-muted-foreground">
                    {formatRelativeTime(c.at)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="snapshots" className="mt-2">
        <SectionCard size="sm" title="Snapshots / time travel">
          {a.snapshots.length === 0 ? (
            <QuietEmpty title="No snapshots for this asset" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.snapshots.map((s) => (
                <li key={s.id} className="flex justify-between gap-2 py-1.5">
                  <span>{s.operation}</span>
                  <span className="text-muted-foreground">
                    {formatRelativeTime(s.committedAt)} ·{" "}
                    {formatCompactNumber(s.records)} records
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>

      <TabsContent value="usage" className="mt-2">
        <SectionCard size="sm" title="Usage (7d)">
          <p className="text-sm">
            {a.usage.queries7d} queries · {a.usage.users7d} users · avg{" "}
            {a.usage.avgLatencyMs} ms
          </p>
          {a.recentQueries.length === 0 ? (
            <QuietEmpty title="No recent queries" />
          ) : (
            <ul className="mt-2 space-y-1.5 text-sm">
              {a.recentQueries.map((q) => (
                <li key={q.id} className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-xs text-muted-foreground">{q.sql}</span>
                  <Link
                    href={`/audit?event=aud-query-${q.id}`}
                    className="ml-auto text-xs text-primary hover:underline"
                  >
                    Audit
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </TabsContent>
    </Tabs>
  )
}
