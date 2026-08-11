"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { FilterToolbar, SearchField } from "@/components/patterns/filter-toolbar"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { assetService } from "@/services"
import type { CatalogNamespace } from "@/services/contracts/assets"

const columns: ColumnDef<CatalogNamespace>[] = [
  {
    key: "name",
    header: "Namespace",
    render: (r) => <span className="font-mono text-sm">{r.name}</span>,
  },
  { key: "desc", header: "Description", render: (r) => r.description },
  { key: "assets", header: "Assets", render: (r) => r.assetCount },
  { key: "owner", header: "Owner", render: (r) => r.owner },
  { key: "engine", header: "Source engine", render: (r) => r.sourceEngine },
  { key: "res", header: "Residency", render: (r) => r.residency },
]

/** Unified catalog namespaces with ownership and residency metadata. */
export function CatalogPage() {
  const state = useService((s) => assetService.listNamespaces(s), [])
  const [search, setSearch] = useState("")
  const [selected, setSelected] = useState<CatalogNamespace | null>(null)

  const rows = useMemo(() => {
    if (state.status !== "success") return []
    const q = search.trim().toLowerCase()
    if (!q) return state.data
    return state.data.filter((ns) =>
      `${ns.name} ${ns.owner} ${ns.description}`.toLowerCase().includes(q)
    )
  }, [state.status, state.data, search])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Catalog"
        description="Namespaces, ownership, residency, and source-engine metadata for governed discovery."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search namespaces..."
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
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
