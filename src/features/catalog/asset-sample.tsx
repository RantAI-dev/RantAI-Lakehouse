"use client"

import * as React from "react"
import Link from "next/link"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useService } from "@/hooks/use-service"
import { assetQueryStudioHref } from "@/lib/asset-query"
import { typeFamily } from "@/lib/column-type"
import { nextSort, sortedOrder, type SortState } from "@/lib/sample-grid"
import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"
import { assetService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import { StandInNotice } from "./asset-stand-in"
import type { IcebergTableState } from "./asset-storage"
import { SampleGrid, type Picked } from "./sample-grid"
import { SampleInspector } from "./sample-inspector"

/**
 * `DATA-15` F1: the tab asks for the API's cap, 1,000 rows, once, and shows
 * them `PAGE_ROWS` at a time. One read means a sort can order every row
 * loaded, which a read per page could not.
 */
const SAMPLE_ROWS = 1000
const PAGE_ROWS = 100

/**
 * The Sample tab: the asset's first rows as the reader would get them from
 * a query — masked and row-filtered — as a data preview: a grid that reads
 * like data, which can be sorted, and an inspector beside it for a picked
 * cell or column. Plan:
 * `docs/superpowers/plans/2026-10-05-sample-tab-data-preview.md`; the
 * 1,000 rows and the pager are `DATA-15` F1
 * (`docs/superpowers/plans/2026-10-16-data-15-asset-page-depth.md`).
 * Anything past a thousand rows is Query Studio's job.
 *
 * The detail body carries five rows. They show until the first answer for
 * the chosen size comes back, so the tab is never blank while it loads; where
 * the deployment cannot fetch more (no `getAssetSample`), they are the sample
 * and there is no size to choose.
 *
 * Sorting is of the rows shown, never of the table, and asks the API for
 * nothing. `iceberg` is the page's own state for the table, as the Schema tab
 * takes it, so a column the inspector opens states the same facts here.
 */
export function AssetSample({ asset: a, iceberg }: { asset: AssetDetail; iceberg: IcebergTableState }) {
  const [page, setPage] = React.useState(0)
  const [sort, setSort] = React.useState<SortState>(null)
  const [picked, setPicked] = React.useState<Picked | null>(null)
  // The inspector reads the table's profile when it first mounts, so it mounts at the first pick and
  // not before: opening the tab asks for no profile. It stays after that, rendering nothing while
  // nothing is picked, so a second pick does not read the table again.
  const [inspecting, setInspecting] = React.useState(false)
  const wrapRef = React.useRef<HTMLDivElement>(null)

  // A reader without `query:read` is told so below; there is nothing to ask for.
  const canLoad = !a.sampleRestricted && Boolean(assetService.getAssetSample)
  const more = useService(
    (s) =>
      canLoad && assetService.getAssetSample
        ? assetService.getAssetSample(a.id, SAMPLE_ROWS, s)
        : Promise.resolve(null),
    [a.id, canLoad]
  )
  const loaded = more.status === "success" ? more.data : null
  // While a larger sample loads, the rows already on screen stay there.
  const rows = loaded ?? a.sample
  const columns = Object.keys(rows[0] ?? {})
  const loading = canLoad && more.status === "loading"

  const schema = React.useMemo(() => new Map(a.schema.map((c) => [c.name, c])), [a.schema])
  const sortColumn = sort ? schema.get(sort.column) : undefined
  const family = sortColumn ? typeFamily(sortColumn.dataType) : undefined
  const order = React.useMemo(
    () =>
      sort
        ? sortedOrder(rows, sort.column, sort.direction, family)
        : rows.map((_, at) => at),
    [rows, sort, family]
  )

  // The page on screen: positions `start` to `end` of the sorted order. A sample that shrank under a
  // later page (the rows are re-read) lands on the last page that still exists.
  const pages = Math.max(1, Math.ceil(order.length / PAGE_ROWS))
  const current = Math.min(page, pages - 1)
  const start = current * PAGE_ROWS
  const shown = order.slice(start, start + PAGE_ROWS)

  // A cell picked in rows that are no longer there (a smaller sample) is not picked.
  const pick = picked && (picked.kind === "column" || picked.at < rows.length) ? picked : null

  const pickIt = (next: Picked) => {
    setPicked(next)
    setInspecting(true)
  }
  const close = (restoreFocus: boolean) => {
    setPicked(null)
    // The control that had focus is gone with the panel; focus goes back to the grid's tab stop.
    if (restoreFocus) wrapRef.current?.querySelector<HTMLElement>('td[role="gridcell"][tabindex="0"]')?.focus()
  }
  const turn = (next: number) => {
    setPage(next)
    // A cell is a place on the page that is going away; a column is not.
    setPicked((p) => (p?.kind === "cell" ? null : p))
  }
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Escape" || pick === null) return
    e.preventDefault()
    const panel = wrapRef.current?.querySelector("[data-slot=sample-inspector]")
    close(panel?.contains(document.activeElement) ?? false)
  }

  // In the card's body, not its header action: that slot cannot shrink, so at phone width the controls
  // left the title and description a few characters wide (the Schema tab's card met the same slot).
  const controls = (
    <div className="mb-2 flex flex-wrap items-center justify-end gap-x-3 gap-y-2">
      <Button size="sm" variant="outline" render={<Link href={assetQueryStudioHref(a)} />}>
        Open in Query Studio
      </Button>
    </div>
  )
  // The larger sample failed but rows are on the page: keep them, say so above them.
  const failedMore = more.status === "error" && rows.length > 0

  return (
    <div className="flex flex-col gap-2">
      <StandInNotice asset={a} />
      <SectionCard
        size="sm"
        title="Sample rows"
        description={
          a.sampleRestricted || rows.length === 0
            ? "Masking and row filters applied."
            : `${loading ? `Loading ${formatNumber(SAMPLE_ROWS)} rows… showing the` : "The"} first ${formatNumber(rows.length)} row${
                rows.length === 1 ? "" : "s"
              }, as you would read them: masking and row filters applied.${
                sort
                  ? ` Sorted by ${sort.column} (${sort.direction === "asc" ? "ascending" : "descending"}), among the ${formatNumber(rows.length)} row${rows.length === 1 ? "" : "s"} loaded, all pages.`
                  : ""
              }`
        }
      >
        {controls}
        {a.sampleRestricted ? (
          <EmptyState
            title="Sample rows need query access"
            description="Rows are data, so they require the query:read permission. Use Request access above to ask for it."
            className="py-4"
          />
        ) : more.status === "error" && rows.length === 0 ? (
          <ErrorState error={more.error} onRetry={more.reload} />
        ) : rows.length === 0 ? (
          loading ? (
            <Skeleton className="h-40 w-full" />
          ) : (
            <EmptyState title="No sample rows available" className="py-4" />
          )
        ) : (
          <>
          {failedMore ? (
            <p role="alert" className="mb-2 flex flex-wrap items-center gap-x-2 text-sm text-destructive">
              {`The larger sample could not be loaded. Showing the ${rows.length} row${rows.length === 1 ? "" : "s"} the page already has.`}
              <Button size="sm" variant="ghost" onClick={more.reload}>
                Retry
              </Button>
            </p>
          ) : null}
          <div ref={wrapRef} onKeyDown={onKeyDown} className="flex flex-col gap-3 xl:flex-row xl:items-start">
            <div className={cn("min-w-0 flex-1", loading && "opacity-60")}>
              <SampleGrid
                columns={columns}
                rows={rows}
                order={shown}
                rowOffset={start}
                schema={schema}
                sort={sort}
                onSort={(column) => {
                  setSort((s) => nextSort(s, column))
                  setPage(0)
                }}
                picked={pick}
                onPick={pickIt}
              />
              {order.length > PAGE_ROWS ? (
                <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
                  <span role="status" className="tabular-nums">
                    {`Rows ${formatNumber(start + 1)}–${formatNumber(start + shown.length)} of ${formatNumber(order.length)}`}
                  </span>
                  <div className="flex items-center gap-1">
                    <Button size="sm" variant="outline" disabled={current === 0} onClick={() => turn(current - 1)}>
                      Previous
                    </Button>
                    <Button size="sm" variant="outline" disabled={current >= pages - 1} onClick={() => turn(current + 1)}>
                      Next
                    </Button>
                  </div>
                </div>
              ) : null}
            </div>
            {inspecting ? (
              <SampleInspector
                asset={a}
                iceberg={iceberg}
                picked={pick}
                rowNumber={pick?.kind === "cell" ? order.indexOf(pick.at) + 1 : 0}
                value={pick?.kind === "cell" ? rows[pick.at][pick.column] : null}
                onClose={() => close(true)}
              />
            ) : null}
          </div>
          </>
        )}
      </SectionCard>
    </div>
  )
}
