"use client"

import * as React from "react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
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
import { PolicyActions } from "./policy-actions"
import { getPolicyColumns } from "./policy-columns"

export function PoliciesPage() {
  const state = useService((s) => governanceService.listPolicies(s), [])
  const [selected, setSelected] = React.useState<Policy | null>(null)
  const tableUrlState = useTableUrlState()
  // `?id=` is how an asset's Access tab links to one policy: open it once
  // the list has it.
  const linkedId = useSearchParams().get("id")
  const opened = React.useRef<string | null>(null)
  React.useEffect(() => {
    if (!linkedId || opened.current === linkedId || state.status !== "success") return
    opened.current = linkedId
    const linked = state.data?.find((p) => p.id === linkedId)
    if (linked) setSelected(linked)
  }, [linkedId, state.status, state.data])

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
        description="Access policies applied to catalog tables."
        // WEB-3 D5: policies are made on a table's Access tab, where the
        // policy engine actually enforces them. The old wizard saved
        // free-text subjects/resources/conditions, so a policy made there
        // was listed as enforced and masked nothing.
        actions={
          <p className="max-w-md text-sm text-muted-foreground">
            Policies are added on a table&apos;s Access tab, where they are
            enforced.{" "}
            <Link href="/catalog" className="text-primary hover:underline">
              Browse the Catalog
            </Link>
          </p>
        }
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No policies"
          description="Policies are added on a table's Access tab, where they are enforced."
          action={
            <Link href="/catalog" className="text-sm text-primary hover:underline">
              Browse the Catalog
            </Link>
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
            <div className="flex flex-wrap gap-2">
              <PolicyActions
                policy={selected}
                onChanged={(change) => {
                  // The drawer shows the policy as it now stands; the list
                  // catches up behind it.
                  setSelected(
                    change === "deleted"
                      ? null
                      : { ...selected, status: change === "enforced" ? "ready" : "draft" }
                  )
                  state.reload()
                }}
              />
            </div>
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}

