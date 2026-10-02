"use client"

import * as React from "react"
import Link from "next/link"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useService } from "@/hooks/use-service"
import { assetQueryStudioHref } from "@/lib/asset-query"
import { cn } from "@/lib/utils"
import { assetService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import { StandInNotice } from "./asset-stand-in"
import { CountToggle } from "./count-toggle"

/** The first is what the detail body already carries; the last is the API's cap. */
const SIZES = [5, 25, 50, 100] as const

/**
 * The Sample tab: the asset's first rows as the reader would get them
 * from a query — masked and row-filtered — five by default, more on
 * request. Anything past a hundred is Query Studio's job.
 */
export function AssetSample({ asset: a }: { asset: AssetDetail }) {
  const [size, setSize] = React.useState<number>(SIZES[0])
  // The detail body's own five need no second request.
  const more = useService(
    (s) =>
      size === SIZES[0] || !assetService.getAssetSample
        ? Promise.resolve(null)
        : assetService.getAssetSample(a.id, size, s),
    [a.id, size]
  )
  const loaded = more.status === "success" ? more.data : null
  // While a larger sample loads, the rows already on screen stay there.
  const rows = loaded ?? a.sample
  const columns = Object.keys(rows[0] ?? {})
  const loading = size !== SIZES[0] && more.status === "loading"

  return (
    <div className="flex flex-col gap-2">
      <StandInNotice asset={a} />
      <SectionCard
        size="sm"
        title="Sample rows"
        description={
          a.sampleRestricted || rows.length === 0
            ? "Masking and row filters applied."
            : `${loading ? `Loading ${size} rows… showing the` : "The"} first ${rows.length} row${
                rows.length === 1 ? "" : "s"
              }, as you would read them: masking and row filters applied.`
        }
        action={
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            {a.sampleRestricted ? null : (
              <CountToggle
                label="Rows"
                ariaLabel="Rows to show"
                options={SIZES}
                value={size}
                onChange={setSize}
              />
            )}
            <Button size="sm" variant="outline" render={<Link href={assetQueryStudioHref(a)} />}>
              Open in Query Studio
            </Button>
          </div>
        }
      >
        {a.sampleRestricted ? (
          <EmptyState
            title="Sample rows need query access"
            description="Rows are data, so they require the query:read permission. Use Request access above to ask for it."
            className="py-4"
          />
        ) : more.status === "error" ? (
          <ErrorState error={more.error} onRetry={more.reload} />
        ) : rows.length === 0 ? (
          <EmptyState title="No sample rows available" className="py-4" />
        ) : (
          <div className={cn("overflow-hidden rounded-lg border border-border", loading && "opacity-60")}>
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  {columns.map((col) => (
                    <TableHead key={col} className="font-mono text-xs font-medium">
                      {col}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((row, i) => (
                  <TableRow key={i}>
                    {columns.map((col) => (
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
    </div>
  )
}
