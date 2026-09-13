"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { HEALTH_LABEL, type Health } from "@/lib/status"
import { connectorService } from "@/services"
import type { Connector } from "@/services/contracts/connectors"
import type { DataTableFilterField } from "@/types/data-table"
import { DIRECTION_LABEL, getConnectorColumns } from "./connectors-columns"

type Direction = Connector["direction"]

const filterFields: DataTableFilterField<Connector>[] = [
  {
    id: "direction",
    label: "Direction",
    options: (Object.keys(DIRECTION_LABEL) as Direction[]).map((d) => ({
      value: d,
      label: DIRECTION_LABEL[d],
    })),
  },
  {
    id: "health",
    label: "Health",
    options: (Object.keys(HEALTH_LABEL) as Health[]).map((h) => ({
      value: h,
      label: HEALTH_LABEL[h],
    })),
  },
]

/** Drawer body — fetches full connector detail for the selected row. */
function ConnectorDetail({ id }: { readonly id: string }) {
  const state = useService((s) => connectorService.getConnector(id, s), [id])
  const testAction = useServiceAction(
    withNotify(
      { success: "Connection test passed", error: "Connection test failed" },
      (signal, connectorId: string) =>
        connectorService.testConnection(connectorId, signal)
    )
  )

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
          { label: "Last test", value: formatRelativeTime(c.lastTestAt) },
          { label: "Last activity", value: formatRelativeTime(c.lastActivityAt) },
          { label: "Discovered assets", value: c.discoveredAssets },
        ]}
      />
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
    </>
  )
}

export function ConnectorsPage() {
  const state = useService((s) => connectorService.listConnectors(s), [])
  const [selected, setSelected] = useState<Connector | null>(null)

  const columns = useMemo(
    () => getConnectorColumns({ onSelect: setSelected }),
    []
  )

  const { table } = useDataTable({
    data: state.data ?? [],
    columns,
    pageCount: 1,
    filterFields,
    enableRowSelection: false,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
    shallow: false,
    clearOnDefault: true,
  })

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
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              table={table}
              placeholder="Search connectors..."
              className="h-8 w-40 lg:w-64"
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
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
