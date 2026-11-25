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
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { governanceService } from "@/services"
import { accessRule, accessTabHref, type AccessRule } from "./access-rules"
import { PolicyNotEnforceableNotice } from "./policy-actions"
import {
  AccessRuleTable,
  PolicyStatusPill,
  getAccessRuleColumns,
} from "./policy-columns"

/**
 * Access rules (`WEB-3` S3-D4, at `/governance/policies`, the address
 * kept): every policy, read-only. A rule is made, enforced, stopped and
 * deleted on its table's Access tab, so this list and its drawer link
 * there instead of offering a second set of buttons.
 */
export function AccessRulesPage() {
  const state = useService((s) => governanceService.listPolicies(s), [])
  const rules = React.useMemo(
    () => (state.status === "success" && state.data ? state.data.map(accessRule) : []),
    [state.status, state.data]
  )
  const [selected, setSelected] = React.useState<AccessRule | null>(null)
  const tableUrlState = useTableUrlState()
  // `?id=` is how an asset's Access tab links to one policy: open it once
  // the list has it.
  const linkedId = useSearchParams().get("id")
  const opened = React.useRef<string | null>(null)
  React.useEffect(() => {
    if (!linkedId || opened.current === linkedId || state.status !== "success") return
    opened.current = linkedId
    const linked = rules.find((p) => p.id === linkedId)
    if (linked) setSelected(linked)
  }, [linkedId, state.status, rules])

  const columns = React.useMemo(
    () => getAccessRuleColumns({ onSelect: setSelected }),
    []
  )

  const filteredData = React.useMemo(() => {
    return filterDataClientSide(rules, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.name,
        (r) => r.table,
        (r) => r.roles,
        (r) => r.does,
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    rules,
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
        title="Access rules"
        description="Who sees which columns masked and which rows filtered, on every table."
        // WEB-3 D5, S3-D4: rules are made and changed on a table's Access
        // tab, where the policy engine enforces them. The old wizard saved
        // free-text subjects/resources/conditions, so a policy made there
        // was listed as enforced and masked nothing.
        actions={
          <p className="max-w-md text-sm text-muted-foreground">
            Rules are added and changed on a table&apos;s Access tab, where
            they are enforced.{" "}
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
          title="No access rules"
          description="Rules are added on a table's Access tab, where they are enforced."
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
            <DataTableSearch placeholder="Search rule, table, roles…" />
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
        description="Access rule"
      >
        {selected ? <AccessRuleDetail rule={selected} /> : null}
      </DetailDrawer>
    </div>
  )
}


/**
 * The drawer: what the rule does, read-only, and where to change it. The
 * Access tab link appears only when the API named the table's asset.
 */
function AccessRuleDetail({ rule }: { rule: AccessRule }) {
  const href = accessTabHref(rule)
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <PolicyStatusPill policy={rule} />
      </div>
      <MetadataList
        items={[
          { label: "Table", value: <AccessRuleTable rule={rule} /> },
          { label: "Roles", value: rule.roles },
          { label: "What it does", value: rule.does },
          { label: "Kind", value: rule.kind },
          { label: "Effect", value: rule.effect },
          { label: "Version", value: `v${rule.version}` },
          { label: "Owner", value: rule.owner },
          { label: "Updated", value: formatRelativeTime(rule.updatedAt) },
        ]}
      />
      {rule.enforceable ? null : <PolicyNotEnforceableNotice />}
      <p className="text-sm text-muted-foreground">
        {href ? (
          <>
            Enforce, stop or delete it on{" "}
            <Link href={href} className="text-primary hover:underline">
              the table&apos;s Access tab
            </Link>
            .
          </>
        ) : (
          <>
            Enforce, stop or delete it on its table&apos;s Access tab, from the{" "}
            <Link href="/catalog" className="text-primary hover:underline">
              Catalog
            </Link>
            .
          </>
        )}
      </p>
    </>
  )
}
