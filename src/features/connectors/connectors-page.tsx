"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { backfillTriggerMessage } from "@/lib/connectors/backfill-message"
import { formatRelativeTime } from "@/lib/format"
import { HEALTH_LABEL, type Health } from "@/lib/status"
import { connectorService } from "@/services"
import type { Connector, IngestRun } from "@/services/contracts/connectors"
import { ConnectorCredentialRotation } from "./connector-credential-rotation"
import { ConnectorProbeHistoryPanel } from "./connector-probe-history-panel"

type Direction = Connector["direction"]

const DIRECTION_LABEL: Record<Direction, string> = {
  source: "Source",
  sink: "Sink",
  bidirectional: "Bidirectional",
}

const columns: ColumnDef<Connector>[] = [
  {
    key: "name",
    header: "Connector",
    render: (r) => (
      <div>
        <p className="font-medium">{r.name}</p>
        <p className="text-xs text-muted-foreground">{r.type}</p>
      </div>
    ),
  },
  {
    key: "dir",
    header: "Direction",
    render: (r) => DIRECTION_LABEL[r.direction],
  },
  {
    key: "health",
    header: "Health",
    render: (r) => <HealthBadge health={r.health} />,
  },
  { key: "env", header: "Environment", render: (r) => r.environment },
  { key: "tenant", header: "Tenant", render: (r) => r.tenant },
  {
    key: "test",
    header: "Last test",
    render: (r) => (
      <span className="text-muted-foreground">
        {r.lastTestAt === null ? "Never tested" : formatRelativeTime(r.lastTestAt)}
      </span>
    ),
  },
  {
    key: "activity",
    header: "Last activity",
    render: (r) => (
      <span className="text-muted-foreground">
        {r.lastActivityAt === null ? "—" : formatRelativeTime(r.lastActivityAt)}
      </span>
    ),
  },
]

/**
 * Read-only `Debezium` `.properties` rendering for a `cdc` adapter's
 * captured table. `properties` holds ONLY `${ENV_VAR_NAME}` references —
 * labeled explicitly as such, never resolved here or anywhere in the
 * console.
 */
function DebeziumPanel({ connectorId, table }: { connectorId: string; table: string }) {
  const props = useService(
    (signal) => connectorService.getDebeziumProperties(connectorId, table, signal),
    [connectorId, table]
  )

  if (props.status === "loading") return <LoadingSkeleton rows={2} />
  if (props.status === "error")
    return <ErrorState error={props.error} onRetry={props.reload} />

  return (
    <div>
      <p className="text-xs font-medium text-muted-foreground">
        Debezium properties · {props.data.table}
      </p>
      <p className="mt-0.5 text-xs text-muted-foreground">{props.data.note}</p>
      <pre className="mt-1.5 overflow-x-auto rounded-md border bg-muted p-2 font-mono text-xs">
        {props.data.properties}
      </pre>
    </div>
  )
}

/**
 * Ingest run history + a manual "Run now" trigger (WS3 item 17/36),
 * `bronze_meta.ingest_run` surfaced through
 * `GET /api/governance/ingest-runs?connectorId=`. For a `cdc` adapter,
 * additionally renders the read-only Debezium properties panel for its
 * first captured table (CDC has no separate batch trigger — see
 * `runNow.data.reason` when `supported` is `false`).
 */
function IngestRunsPanel({ connectorId }: { connectorId: string }) {
  const spec = useService((s) => connectorService.getIngestSpec(connectorId, s), [connectorId])
  const runs = useService(
    (s) => connectorService.listIngestRuns(connectorId, s),
    [connectorId]
  )
  const runNow = useServiceAction((signal, id: string) => connectorService.runIngest(id, signal))

  if (spec.status === "loading" || runs.status === "loading") return <LoadingSkeleton rows={3} />
  if (spec.status === "error")
    return <ErrorState error={spec.error} onRetry={spec.reload} />
  if (runs.status === "error")
    return <ErrorState error={runs.error} onRetry={runs.reload} />

  const table: string | undefined = spec.data.sourceObjects[0]?.name
  // `driver` names the CDC source for the honest, generic message below
  // only — the dial's actual shape is validated server-side
  // (`Dial::parse`, `rust/crates/lakehouse-store/src/ingest_spec.rs`),
  // never re-validated here (see `Dial`'s doc comment in
  // `@/services/contracts/connectors`).
  const driver =
    spec.data.adapter === "cdc" && typeof spec.data.dial.driver === "string"
      ? spec.data.dial.driver
      : "cdc"

  return (
    <div className="space-y-3">
      {spec.data.adapter === "cdc" ? (
        <p className="text-sm text-muted-foreground">{backfillTriggerMessage(driver)}</p>
      ) : null}
      <div className="flex items-center justify-between">
        <p className="text-xs font-medium text-muted-foreground">Ingest runs</p>
        <Button
          size="sm"
          variant="outline"
          disabled={runNow.status === "pending"}
          onClick={async () => {
            await runNow.run(connectorId)
            runs.reload()
          }}
        >
          {runNow.status === "pending" ? "Running…" : "Run now"}
        </Button>
      </div>
      {runNow.data && runNow.data.supported === false ? (
        <p className="text-sm text-muted-foreground">Not runnable · {runNow.data.reason}</p>
      ) : null}
      {runs.data.length === 0 ? (
        <p className="text-sm text-muted-foreground">No ingest runs yet.</p>
      ) : (
        <ul className="space-y-1 text-sm">
          {runs.data.map((r: IngestRun) => (
            <li key={`${r.job}-${r.startedAt}`}>
              {r.object}: {r.status} ({r.rows === null ? "—" : r.rows} rows)
              <span className="ml-2 text-xs text-muted-foreground">
                {formatRelativeTime(r.startedAt)}
              </span>
            </li>
          ))}
        </ul>
      )}
      {spec.data.adapter === "cdc" && table ? (
        <DebeziumPanel connectorId={connectorId} table={table} />
      ) : null}
    </div>
  )
}

/** Drawer body — fetches full connector detail for the selected row. */
function ConnectorDetail({ id }: { id: string }) {
  const state = useService((s) => connectorService.getConnector(id, s), [id])
  const testAction = useServiceAction((signal, connectorId: string) =>
    connectorService.testConnection(connectorId, signal)
  )
  const [historyKey, setHistoryKey] = useState(0)

  if (state.status === "loading") return <LoadingSkeleton rows={4} />
  if (state.status === "error")
    return <ErrorState error={state.error} onRetry={state.reload} />
  const c = state.data

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <HealthBadge health={c.health} />
        <Pill tone="neutral">{DIRECTION_LABEL[c.direction]}</Pill>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={testAction.status === "pending"}
          onClick={async () => {
            await testAction.run(id)
            state.reload()
            setHistoryKey((k) => k + 1)
          }}
        >
          {testAction.status === "pending" ? "Testing…" : "Test connection"}
        </Button>
        <Button size="sm" render={<Link href={`/pipelines/create?connectorId=${id}`} />}>
          Create pipeline
        </Button>
        {c.auditEventId ? (
          <Button
            size="sm"
            variant="ghost"
            render={<Link href={`/audit?event=${c.auditEventId}`} />}
          >
            Audit
          </Button>
        ) : null}
      </div>
      {testAction.data ? (
        <p
          className={
            !testAction.data.supported
              ? "text-sm text-muted-foreground"
              : testAction.data.ok
                ? "text-sm text-emerald-600 dark:text-emerald-400"
                : "text-sm text-destructive"
          }
        >
          {testAction.data.supported ? (
            <>
              {testAction.data.message}
              {testAction.data.latencyMs !== null ? ` · ${testAction.data.latencyMs} ms` : ""}
            </>
          ) : (
            <>Not testable · {testAction.data.message}</>
          )}
        </p>
      ) : null}
      <MetadataList
        items={[
          { label: "Type", value: c.type },
          { label: "Direction", value: DIRECTION_LABEL[c.direction] },
          { label: "Environment", value: c.environment },
          { label: "Tenant", value: c.tenant },
          { label: "Owner", value: c.owner },
          {
            label: "Last test",
            value: c.lastTestAt === null ? "Never tested" : formatRelativeTime(c.lastTestAt),
          },
          {
            label: "Last activity",
            value: c.lastActivityAt === null ? "—" : formatRelativeTime(c.lastActivityAt),
          },
          { label: "Discovered assets", value: c.discoveredAssets },
        ]}
      />
      <ConnectorProbeHistoryPanel connectorId={id} refreshKey={historyKey} />
      <ConnectorCredentialRotation connectorId={id} onRotated={state.reload} />
      <div>
        <p className="text-xs font-medium text-muted-foreground">Capabilities</p>
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          {c.capabilities.map((cap) => (
            <Pill key={cap} tone="neutral">
              {cap}
            </Pill>
          ))}
        </div>
      </div>
      <div>
        <p className="text-xs font-medium text-muted-foreground">
          Discovered schemas
        </p>
        {c.discoveredSchemas.length === 0 ? (
          <p className="mt-1 text-sm text-muted-foreground">
            No schemas discovered yet.
          </p>
        ) : (
          <ul className="mt-1 space-y-1">
            {c.discoveredSchemas.map((s) => (
              <li key={s.name} className="font-mono text-sm">
                {s.name}
                <span className="ml-2 text-xs text-muted-foreground">
                  {s.kind}
                  {s.columnsOrFields > 0 ? ` · ${s.columnsOrFields} fields` : ""}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div>
        <p className="text-xs font-medium text-muted-foreground">Recent errors</p>
        {c.recentErrors.length === 0 ? (
          <p className="mt-1 text-sm text-muted-foreground">No recent errors.</p>
        ) : (
          <ul className="mt-1 space-y-1.5">
            {c.recentErrors.map((err, i) => (
              <li key={i} className="text-sm text-destructive">
                {err.message}
                <span className="ml-2 text-xs text-muted-foreground">
                  {formatRelativeTime(err.at)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div>
        <p className="text-xs font-medium text-muted-foreground">
          Dependent workloads
        </p>
        {c.dependentPipelines.length === 0 ? (
          <p className="mt-1 text-sm text-muted-foreground">
            No dependent pipelines.
          </p>
        ) : (
          <ul className="mt-1 space-y-1">
            {c.dependentPipelines.map((p) => (
              <li key={p.id}>
                <Link
                  href={`/pipelines/${p.id}`}
                  className="font-mono text-sm text-primary hover:underline"
                >
                  {p.name}
                </Link>
                <span className="ml-2 text-xs text-muted-foreground">{p.kind}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <IngestRunsPanel connectorId={id} />
    </>
  )
}

export function ConnectorsPage() {
  const state = useService((s) => connectorService.listConnectors(s), [])
  const [search, setSearch] = useState("")
  const [direction, setDirection] = useState<Direction | "all">("all")
  const [health, setHealth] = useState<Health | "all">("all")
  const [selected, setSelected] = useState<Connector | null>(null)

  const rows = useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    return state.data.filter((c) => {
      if (direction !== "all" && c.direction !== direction) return false
      if (health !== "all" && c.health !== health) return false
      if (q) {
        const hay = `${c.name} ${c.type} ${c.tenant} ${c.owner}`.toLowerCase()
        if (!hay.includes(q)) return false
      }
      return true
    })
  }, [state.status, state.data, search, direction, health])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Connectors"
        description="Sources and sinks for CDC, messaging, object storage, SaaS, and federation. Data enters the platform here before processing."
        actions={
          <Button size="sm" render={<Link href="/connectors/create" />}>
            <PlusIcon data-icon="inline-start" />
            New Connector
          </Button>
        }
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search connectors..."
        />
        <FilterSelect
          ariaLabel="Filter by direction"
          allLabel="All directions"
          value={direction}
          onChange={(v) => setDirection(v as Direction | "all")}
          options={Object.entries(DIRECTION_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by health"
          allLabel="All health"
          value={health}
          onChange={(v) => setHealth(v as Health | "all")}
          options={Object.entries(HEALTH_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No connectors"
          description="Add a source or sink to start ingesting and delivering data."
          action={
            <Button size="sm" render={<Link href="/connectors/create" />}>
              New Connector
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) > 0 ? (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(r) => r.id}
          onRowClick={setSelected}
        />
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description={selected?.type}
      >
        {selected ? <ConnectorDetail id={selected.id} /> : null}
      </DetailDrawer>
    </div>
  )
}
