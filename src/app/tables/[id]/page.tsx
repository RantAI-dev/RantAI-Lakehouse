import Link from "next/link"
import { Download, Play } from "lucide-react"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { getTableDetail } from "@/lib/table-fixtures"

/**
 * Server-rendered detail page for a single table at `/tables/[id]`.
 *
 * Fetches fixture data via `getTableDetail(id)` and renders a header with
 * breadcrumbs and actions, summary metrics (rows, size, columns, last update),
 * a schema table, format chips, and a sample-row preview.
 *
 * Note: `params` is awaited because Next.js 15+ exposes `params` as a Promise
 * inside server components.
 */
export default async function TableDetailPage({
  params,
}: {
  params: Promise<{ id: string }>
}) {
  const { id } = await params
  const detail = getTableDetail(id)
  const { name, schema, summary, metadata, preview, formats } = detail

  return (
    <div className="flex flex-col gap-4 p-4">
      {/* Header: breadcrumb + title + actions */}
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
        <div className="flex flex-col gap-1">
          <Breadcrumb>
            <BreadcrumbList className="text-sm text-muted-foreground">
              <BreadcrumbItem>
                <BreadcrumbLink href="/" render={<Link href="/" />}>
                  Dashboard
                </BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem>
                <BreadcrumbPage className="font-normal text-primary">
                  Detail Table
                </BreadcrumbPage>
              </BreadcrumbItem>
            </BreadcrumbList>
          </Breadcrumb>
          <div className="flex items-end gap-1">
            <span className="text-[20px] font-normal leading-7 tracking-[-0.12px] text-primary">
              Detail
            </span>
            <span className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
              {name}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <Button
            variant="outline"
            size="default"
            className="h-10 gap-2 rounded-md border-border px-4 text-sm font-medium text-primary hover:bg-muted hover:text-primary"
          >
            <Download className="size-4" />
            Export
          </Button>
          <Button
            size="default"
            className="h-10 gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground"
          >
            <Play className="size-4" />
            Query
          </Button>
        </div>
      </header>

      {/* List Schema title */}
      <h2 className="text-[20px] font-medium leading-7 tracking-[-0.12px] text-primary">
        List Schema
      </h2>

      {/* Two columns: schema table + summary/metadata cards */}
      <div className="flex gap-3">
        {/* Left: Schema table */}
        <div className="min-w-0 flex-1 overflow-hidden rounded-md border border-border">
          <Table>
            <TableHeader>
              <TableRow className="border-border hover:bg-transparent">
                <TableHead className="h-12 px-4 text-sm font-medium tracking-[-0.084px] text-muted-foreground">
                  Column
                </TableHead>
                <TableHead className="h-12 px-4 text-sm font-medium tracking-[-0.084px] text-muted-foreground">
                  Type
                </TableHead>
                <TableHead className="h-12 px-4 text-sm font-medium tracking-[-0.084px] text-muted-foreground">
                  Nullable
                </TableHead>
                <TableHead className="h-12 px-4 text-sm font-medium tracking-[-0.084px] text-muted-foreground">
                  Description
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {schema.map((row, i) => (
                <TableRow key={i} className="border-border">
                  <TableCell className="h-16 px-4 py-3 text-sm font-normal leading-5 text-primary">
                    {row.column}
                  </TableCell>
                  <TableCell className="h-16 px-4 py-3 text-sm font-normal leading-5 text-primary">
                    {row.type}
                  </TableCell>
                  <TableCell className="h-16 px-4 py-3 text-sm font-normal leading-5 text-primary">
                    {row.nullable}
                  </TableCell>
                  <TableCell className="h-16 px-4 py-3 text-sm font-normal leading-5 text-primary">
                    {row.description}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>

        {/* Right: Summary Data + Metadata cards */}
        <div className="flex w-[236px] shrink-0 flex-col gap-3">
          <Card className="gap-1 overflow-hidden rounded-md border-border bg-card p-3 py-3 shadow-[0px_4px_6px_0px_rgba(0,0,0,0.1),0px_2px_4px_0px_rgba(0,0,0,0.1)]">
            <CardHeader className="p-0">
              <p className="text-base font-medium leading-6 text-muted-foreground">
                Summary Data
              </p>
            </CardHeader>
            <CardContent className="flex flex-col gap-1 p-0 pt-1">
              <SummaryRow value={summary.rows} label="ROWS" />
              <SummaryRow value={summary.size} label="Size" />
              <SummaryRow value={String(summary.columns)} label="Columns" />
              <SummaryRow value={summary.lastUpdate} label="Last Update" />
            </CardContent>
          </Card>
          <Card className="gap-1 overflow-hidden rounded-md border-border bg-card p-3 py-3 shadow-[0px_4px_6px_0px_rgba(0,0,0,0.1),0px_2px_4px_0px_rgba(0,0,0,0.1)]">
            <CardHeader className="p-0">
              <p className="text-base font-medium leading-6 text-muted-foreground">
                Metadata
              </p>
            </CardHeader>
            <CardContent className="flex flex-col gap-1 p-0 pt-1">
              <SummaryRow value={String(metadata.partitions)} label="Partitions" />
              <SummaryRow value={metadata.avgRowSize} label="Avg row size" />
              <SummaryRow value={metadata.format} label="Primary format" />
              <SummaryRow value={metadata.compression} label="Compression" />
              <div className="rounded-lg px-3 py-1.5">
                <p className="text-xs leading-4 tracking-[-0.072px] text-muted-foreground">
                  Open formats
                </p>
                <div className="mt-1 flex flex-wrap gap-1">
                  {formats.map((f) => (
                    <Badge
                      key={f}
                      variant="outline"
                      className="border-primary/30 text-[11px] font-medium text-primary"
                    >
                      {f}
                    </Badge>
                  ))}
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>

      <h2 className="mt-6 text-[20px] font-medium leading-7 tracking-[-0.12px] text-primary">
        Sample preview
      </h2>
      <p className="mb-3 text-sm text-muted-foreground">
        First rows from the latest table snapshot. Run full scans in Query Studio.
      </p>
      <div className="overflow-hidden rounded-md border border-border">
        <Table>
          <TableHeader>
            <TableRow className="border-border hover:bg-transparent">
              <TableHead className="h-11 px-4 text-sm font-medium text-muted-foreground">
                customer_id
              </TableHead>
              <TableHead className="h-11 px-4 text-sm font-medium text-muted-foreground">
                name
              </TableHead>
              <TableHead className="h-11 px-4 text-sm font-medium text-muted-foreground">
                portfolio
              </TableHead>
              <TableHead className="h-11 px-4 text-sm font-medium text-muted-foreground">
                as_of
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {preview.map((row, i) => (
              <TableRow key={i} className="border-border">
                <TableCell className="px-4 py-3 font-mono text-xs text-primary">
                  {row.col1}
                </TableCell>
                <TableCell className="px-4 py-3 text-sm text-primary">
                  {row.col2}
                </TableCell>
                <TableCell className="px-4 py-3 text-sm text-muted-foreground">
                  {row.col3}
                </TableCell>
                <TableCell className="px-4 py-3 font-mono text-xs text-muted-foreground">
                  {row.col4}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  )
}

function SummaryRow({
  value,
  label,
  valueSuffix,
}: {
  value: string
  label: string
  valueSuffix?: string
}) {
  return (
    <div className="flex flex-col gap-1 rounded-lg px-3 py-1.5">
      <div className="flex items-end gap-1">
        <span className="text-base font-medium leading-none tracking-[-0.096px] text-primary">
          {value}
        </span>
        {valueSuffix && (
          <span className="text-sm tracking-[-0.084px] text-muted-foreground">
            {valueSuffix}
          </span>
        )}
      </div>
      <p className="text-xs leading-4 tracking-[-0.072px] text-muted-foreground">
        {label}
      </p>
    </div>
  )
}
