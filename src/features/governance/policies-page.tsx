"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { governanceService } from "@/services"
import type { Policy } from "@/services/contracts/governance"
import { getPolicyColumns } from "./policy-columns"

export function PoliciesPage() {
  const state = useService((s) => governanceService.listPolicies(s), [])
  const [selected, setSelected] = React.useState<Policy | null>(null)
  const tableUrlState = useTableUrlState()

  const columns = React.useMemo(
    () => getPolicyColumns({ onSelect: setSelected }),
    []
  )

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.name,
        (r) => r.subjects,
        (r) => r.resources,
        (r) => r.kind,
        (r) => r.effect,
        (r) => r.owner,
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    state.status,
    state.data,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/governance/policies",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Policies"
        description="Access, agent, residency, retention, and budget policies."
        actions={
          <Button size="sm" render={<Link href="/governance/policies/create" />}>
            <PlusIcon data-icon="inline-start" />
            Create Policy
          </Button>
        }
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No policies"
          description="Create an access, agent, residency, retention, or budget policy."
          action={
            <Button
              size="sm"
              render={<Link href="/governance/policies/create" />}
            >
              Create Policy
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) > 0 ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search name, subjects, resources, kind…" />
          </DataTableAdvancedToolbar>
          <DataTable table={table} onRowClick={setSelected} />
        </div>
      ) : null}

      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description="Policy detail"
      >
        {selected ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge status={selected.status} />
            </div>
            <MetadataList
              items={[
                { label: "Kind", value: selected.kind },
                { label: "Subjects", value: selected.subjects },
                { label: "Resources", value: selected.resources },
                { label: "Effect", value: selected.effect },
                { label: "Version", value: `v${selected.version}` },
                { label: "Owner", value: selected.owner },
                {
                  label: "Updated",
                  value: formatRelativeTime(selected.updatedAt),
                },
              ]}
            />
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}

