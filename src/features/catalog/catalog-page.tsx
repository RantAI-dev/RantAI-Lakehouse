"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { assetService } from "@/services"
import type { CatalogNamespace } from "@/services/contracts/assets"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { getCatalogNamespaceColumns } from "./catalog-namespace-columns"

/** Unified catalog namespaces with ownership and residency metadata. */
export function CatalogPage() {
  const state = useService((s) => assetService.listNamespaces(s), [])
  const [selected, setSelected] = useState<CatalogNamespace | null>(null)
  const tableUrlState = useTableUrlState()

  const columns = useMemo(
    () => getCatalogNamespaceColumns({ onSelect: setSelected }),
    []
  )

  const filteredData = useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (ns) => ns.name,
        (ns) => ns.owner,
        (ns) => ns.description,
        (ns) => ns.sourceEngine,
        (ns) => ns.residency,
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
    persistKey: "/catalog",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Catalog"
        description="Namespaces, ownership, residency, and source-engine metadata for governed discovery."
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search namespaces…" />
          </DataTableAdvancedToolbar>
          <DataTable table={table} onRowClick={setSelected} />
        </div>
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
      >
        {selected ? (
          <>
            <p className="text-sm text-foreground">{selected.description}</p>
            <MetadataList
              items={[
                { label: "Owner", value: selected.owner },
                { label: "Source engine", value: selected.sourceEngine },
                { label: "Residency", value: selected.residency },
                { label: "Assets", value: selected.assetCount },
              ]}
            />
            {/* Data Explorer reads its search term from the `q` param and the
                mock asset search matches on namespace. */}
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              render={<Link href={`/data?q=${encodeURIComponent(selected.name)}`} />}
            >
              Browse assets
            </Button>
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
