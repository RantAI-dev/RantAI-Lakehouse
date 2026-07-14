"use client"

import { useCallback, useMemo, useState } from "react"
import Link from "next/link"
import {
  ChevronRight,
  ExternalLink,
  Library,
  Search,
  Shield,
  Table2,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Separator } from "@/components/ui/separator"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { cn } from "@/lib/utils"

type Asset = {
  id: string
  fqn: string
  name: string
  zone: string
  owner: string
  classification: string
  permission: "Read" | "Read/Write" | "None"
}

const ASSETS: Asset[] = [
  {
    id: "1",
    fqn: "prod.gold.orders_fact",
    name: "gold.orders_fact",
    zone: "Gold",
    owner: "Data Platform",
    classification: "Confidential",
    permission: "Read",
  },
  {
    id: "2",
    fqn: "prod.silver.dim_customer",
    name: "silver.dim_customer",
    zone: "Silver",
    owner: "Analytics",
    classification: "Restricted",
    permission: "Read/Write",
  },
  {
    id: "3",
    fqn: "prod.raw.ingest.orders",
    name: "raw.ingest.orders",
    zone: "Raw",
    owner: "Ingestion",
    classification: "Internal",
    permission: "Read",
  },
  {
    id: "4",
    fqn: "prod.semantic.support_embeddings",
    name: "semantic.support_embeddings",
    zone: "Semantic",
    owner: "ML Platform",
    classification: "Restricted",
    permission: "Read",
  },
]

type ColumnDef = {
  name: string
  type: string
  nullable: string
  description: string
}

type CatalogTableDetail = {
  fqn: string
  catalog: string
  schema: string
  table: string
  owner: string
  zone: string
  classification: string
  permission: Asset["permission"]
  rowEstimate: string
  logicalSize: string
  format: string
  columnCount: number
  lastRefreshed: string
  description: string
  tags: string[]
  columns: ColumnDef[]
  /** When set, links to existing table detail page in the app */
  tableDetailHref?: string
}

const DETAIL_BY_FQN: Record<string, CatalogTableDetail> = {
  "prod.gold.orders_fact": {
    fqn: "prod.gold.orders_fact",
    catalog: "prod",
    schema: "gold",
    table: "orders_fact",
    owner: "Data Platform",
    zone: "Gold",
    classification: "Confidential",
    permission: "Read",
    rowEstimate: "5.1 M",
    logicalSize: "2.4 GB",
    format: "Parquet",
    columnCount: 10,
    lastRefreshed: "30 min ago",
    description:
      "Curated order line facts with dimensions for revenue and fulfillment analytics.",
    tags: ["finance", "fact", "partitioned"],
    tableDetailHref: "/tables/6",
    columns: [
      {
        name: "order_id",
        type: "STRING",
        nullable: "No",
        description: "Business order identifier",
      },
      {
        name: "customer_id",
        type: "STRING",
        nullable: "No",
        description: "Customer key",
      },
      {
        name: "order_date",
        type: "DATE",
        nullable: "No",
        description: "Order date (UTC)",
      },
      {
        name: "amount",
        type: "DECIMAL(18,2)",
        nullable: "No",
        description: "Order total amount",
      },
      {
        name: "tenant_id",
        type: "STRING",
        nullable: "No",
        description: "Tenant scope for RLS",
      },
    ],
  },
  "prod.gold.revenue_agg": {
    fqn: "prod.gold.revenue_agg",
    catalog: "prod",
    schema: "gold",
    table: "revenue_agg",
    owner: "Finance Ops",
    zone: "Gold",
    classification: "Confidential",
    permission: "Read",
    rowEstimate: "842 K",
    logicalSize: "410 MB",
    format: "Delta",
    columnCount: 14,
    lastRefreshed: "2 h ago",
    description:
      "Monthly revenue aggregates by region and product line for reporting.",
    tags: ["finance", "aggregate"],
    columns: [
      {
        name: "period",
        type: "DATE",
        nullable: "No",
        description: "Aggregation month",
      },
      {
        name: "region",
        type: "STRING",
        nullable: "No",
        description: "Sales region",
      },
      {
        name: "revenue",
        type: "DECIMAL(18,2)",
        nullable: "No",
        description: "Recognized revenue",
      },
    ],
  },
  "prod.gold.gl_entries": {
    fqn: "prod.gold.gl_entries",
    catalog: "prod",
    schema: "gold",
    table: "gl_entries",
    owner: "Finance Ops",
    zone: "Gold",
    classification: "Restricted",
    permission: "Read",
    rowEstimate: "12 M",
    logicalSize: "3.1 GB",
    format: "Iceberg",
    columnCount: 22,
    lastRefreshed: "6 h ago",
    description: "General ledger entries joined to chart of accounts.",
    tags: ["finance", "gl", "compliance"],
    columns: [
      {
        name: "entry_id",
        type: "STRING",
        nullable: "No",
        description: "GL entry id",
      },
      {
        name: "entity_code",
        type: "STRING",
        nullable: "No",
        description: "Legal entity",
      },
      {
        name: "amount",
        type: "DECIMAL(24,4)",
        nullable: "No",
        description: "Posted amount",
      },
    ],
  },
  "prod.silver.dim_customer": {
    fqn: "prod.silver.dim_customer",
    catalog: "prod",
    schema: "silver",
    table: "dim_customer",
    owner: "Analytics",
    zone: "Silver",
    classification: "Restricted",
    permission: "Read/Write",
    rowEstimate: "2.8 M",
    logicalSize: "1.2 GB",
    format: "Parquet",
    columnCount: 18,
    lastRefreshed: "1 h ago",
    description:
      "Conformed customer dimension with PII columns masked by policy.",
    tags: ["dimension", "pii"],
    tableDetailHref: "/tables/2",
    columns: [
      {
        name: "customer_id",
        type: "STRING",
        nullable: "No",
        description: "Surrogate key",
      },
      {
        name: "legal_name",
        type: "STRING",
        nullable: "Yes",
        description: "Masked in query",
      },
      {
        name: "segment",
        type: "STRING",
        nullable: "Yes",
        description: "Marketing segment",
      },
    ],
  },
  "prod.silver.orders_clean": {
    fqn: "prod.silver.orders_clean",
    catalog: "prod",
    schema: "silver",
    table: "orders_clean",
    owner: "Data Platform",
    zone: "Silver",
    classification: "Internal",
    permission: "Read",
    rowEstimate: "4.9 M",
    logicalSize: "1.9 GB",
    format: "Parquet",
    columnCount: 12,
    lastRefreshed: "45 min ago",
    description: "Cleansed orders after bronze validation rules.",
    tags: ["orders", "cleansed"],
    columns: [
      {
        name: "order_id",
        type: "STRING",
        nullable: "No",
        description: "Order id",
      },
      {
        name: "status",
        type: "STRING",
        nullable: "No",
        description: "Lifecycle status",
      },
    ],
  },
  "prod.semantic.support_embeddings": {
    fqn: "prod.semantic.support_embeddings",
    catalog: "prod",
    schema: "semantic",
    table: "support_embeddings",
    owner: "ML Platform",
    zone: "Semantic",
    classification: "Restricted",
    permission: "Read",
    rowEstimate: "890 K",
    logicalSize: "842 MB",
    format: "Parquet + vector",
    columnCount: 6,
    lastRefreshed: "4 h ago",
    description:
      "Vector index over support ticket text; used by Search & similarity.",
    tags: ["semantic", "embedding", "vector"],
    columns: [
      {
        name: "ticket_id",
        type: "STRING",
        nullable: "No",
        description: "Source ticket id",
      },
      {
        name: "embedding",
        type: "VECTOR(1536)",
        nullable: "No",
        description: "Embedding vector",
      },
      {
        name: "model_version",
        type: "STRING",
        nullable: "No",
        description: "Embedding model id",
      },
    ],
  },
  "prod.raw.ingest.orders": {
    fqn: "prod.raw.ingest.orders",
    catalog: "prod",
    schema: "raw",
    table: "ingest.orders",
    owner: "Ingestion",
    zone: "Raw",
    classification: "Internal",
    permission: "Read",
    rowEstimate: "18 M",
    logicalSize: "9.2 GB",
    format: "JSON / Parquet",
    columnCount: 9,
    lastRefreshed: "15 min ago",
    description: "Raw landing copy of upstream order payloads before typing.",
    tags: ["raw", "ingest"],
    columns: [
      {
        name: "payload_id",
        type: "STRING",
        nullable: "No",
        description: "Ingest id",
      },
      {
        name: "body",
        type: "JSON",
        nullable: "No",
        description: "Raw payload",
      },
    ],
  },
}

const TREE = [
  {
    catalog: "prod",
    schemas: [
      {
        name: "gold",
        tables: ["orders_fact", "revenue_agg", "gl_entries"],
      },
      { name: "silver", tables: ["dim_customer", "orders_clean"] },
      { name: "semantic", tables: ["support_embeddings"] },
    ],
  },
] as const

function fqnFor(catalog: string, schema: string, table: string): string {
  return `${catalog}.${schema}.${table}`
}

function getDetailForFqn(fqn: string): CatalogTableDetail | null {
  return DETAIL_BY_FQN[fqn] ?? null
}

export default function DataCatalogPage() {
  const [q, setQ] = useState("")
  const [openCatalog, setOpenCatalog] = useState<string | null>("prod")
  const [openSchema, setOpenSchema] = useState<string | null>("gold")

  const [detailOpen, setDetailOpen] = useState(false)
  const [activeDetail, setActiveDetail] = useState<CatalogTableDetail | null>(
    null
  )

  const openTableDetail = useCallback((fqn: string) => {
    const d = getDetailForFqn(fqn)
    if (d) {
      setActiveDetail(d)
      setDetailOpen(true)
    }
  }, [])

  const filtered = useMemo(() => {
    const s = q.trim().toLowerCase()
    if (!s) return ASSETS
    return ASSETS.filter(
      (a) =>
        a.name.toLowerCase().includes(s) ||
        a.fqn.toLowerCase().includes(s) ||
        a.zone.toLowerCase().includes(s) ||
        a.owner.toLowerCase().includes(s)
    )
  }, [q])

  return (
    <div className="flex flex-col gap-4">
      <header className="border-b border-border pb-2">
        <div className="flex flex-wrap items-center gap-2">
          <Library className="size-7 text-primary" aria-hidden />
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Data catalog
          </h1>
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          Browse catalogs, schemas, and tables with governance tags and access.
          Select a table to view columns and metadata.
        </p>
      </header>

      <div className="grid gap-4 lg:grid-cols-[minmax(0,300px)_1fr]">
        <Card className="rounded-lg border border-border bg-card shadow-sm">
          <CardHeader className="border-b border-border px-4 py-3">
            <h2 className="text-base font-medium text-primary">Browse</h2>
            <p className="text-sm text-muted-foreground">
              Catalog → schema → table (click a table for detail)
            </p>
          </CardHeader>
          <CardContent className="p-2">
            {TREE.map((cat) => (
              <div key={cat.catalog} className="rounded-md">
                <button
                  type="button"
                  className="flex w-full items-center gap-1 rounded-md px-2 py-1.5 text-left text-sm font-medium text-primary hover:bg-muted/80"
                  onClick={() =>
                    setOpenCatalog((c) => (c === cat.catalog ? null : cat.catalog))
                  }
                >
                  <ChevronRight
                    className={cn(
                      "size-4 shrink-0 transition-transform",
                      openCatalog === cat.catalog && "rotate-90"
                    )}
                  />
                  {cat.catalog}
                </button>
                {openCatalog === cat.catalog &&
                  cat.schemas.map((sch) => (
                    <div key={sch.name} className="ml-5 border-l border-border pl-2">
                      <button
                        type="button"
                        className={cn(
                          "mt-1 flex w-full items-center gap-1 rounded-md px-2 py-1 text-left text-xs font-medium hover:bg-muted/80",
                          openSchema === sch.name
                            ? "text-primary"
                            : "text-muted-foreground"
                        )}
                        onClick={() =>
                          setOpenSchema((s) => (s === sch.name ? null : sch.name))
                        }
                      >
                        {sch.name}
                      </button>
                      {openSchema === sch.name && (
                        <ul className="ml-3 mt-1 space-y-0.5 border-l border-border pl-2">
                          {sch.tables.map((t) => {
                            const fqn = fqnFor(cat.catalog, sch.name, t)
                            const hasDetail = Boolean(getDetailForFqn(fqn))
                            return (
                              <li key={t}>
                                <button
                                  type="button"
                                  disabled={!hasDetail}
                                  onClick={() => hasDetail && openTableDetail(fqn)}
                                  className={cn(
                                    "flex h-8 w-full items-center gap-1.5 rounded-md px-2 text-left font-mono text-[11px]",
                                    hasDetail
                                      ? "cursor-pointer text-primary hover:bg-primary/10"
                                      : "cursor-not-allowed text-muted-foreground opacity-60"
                                  )}
                                >
                                  <Table2 className="size-3.5 shrink-0 opacity-70" />
                                  {t}
                                </button>
                              </li>
                            )
                          })}
                        </ul>
                      )}
                    </div>
                  ))}
              </div>
            ))}
          </CardContent>
        </Card>

        <div className="flex flex-col gap-3">
          <div className="relative max-w-md">
            <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-primary" />
            <Input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="Search assets…"
              className="h-10 rounded-md border-border pl-9"
            />
          </div>

          <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-sm">
            <CardHeader className="border-b border-border px-4 py-3">
              <div className="flex items-center gap-2">
                <Shield className="size-4 text-primary" aria-hidden />
                <h2 className="text-base font-medium text-primary">Assets</h2>
              </div>
              <p className="text-xs text-muted-foreground">
                Click a row to open the full table detail.
              </p>
            </CardHeader>
            <CardContent className="divide-y divide-border p-0">
              {filtered.length === 0 ? (
                <p className="p-6 text-sm text-muted-foreground">
                  No assets match your search.
                </p>
              ) : (
                filtered.map((a) => (
                  <button
                    key={a.id}
                    type="button"
                    onClick={() => openTableDetail(a.fqn)}
                    className="flex w-full flex-col gap-2 px-4 py-3 text-left transition-colors hover:bg-muted/50 sm:flex-row sm:items-center sm:justify-between"
                  >
                    <div>
                      <p className="font-mono text-sm font-medium text-primary">
                        {a.name}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {a.fqn} · Owner: {a.owner}
                      </p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge
                        variant="outline"
                        className="border-primary/30 text-[11px] text-primary"
                      >
                        {a.zone}
                      </Badge>
                      <Badge className="border-0 bg-primary/10 text-[11px] text-primary hover:bg-primary/10">
                        {a.classification}
                      </Badge>
                      <Badge
                        variant="outline"
                        className="text-[11px] text-muted-foreground"
                      >
                        {a.permission}
                      </Badge>
                      <ChevronRight className="size-4 text-muted-foreground" />
                    </div>
                  </button>
                ))
              )}
            </CardContent>
          </Card>
        </div>
      </div>

      <Sheet open={detailOpen} onOpenChange={setDetailOpen}>
        <SheetContent
          side="right"
          showCloseButton
          className="h-full max-h-[100dvh] w-full gap-0 border-l border-border p-0 sm:max-w-lg"
        >
          {activeDetail && (
            <div className="flex h-full min-h-0 flex-col">
              <SheetHeader className="shrink-0 space-y-3 border-b border-border bg-muted/20 px-5 pb-4 pt-5 text-left">
                <nav
                  className="flex flex-wrap items-center gap-x-1.5 gap-y-1 font-mono text-[11px] text-muted-foreground"
                  aria-label="Table path"
                >
                  <span className="rounded bg-background px-1.5 py-0.5 text-foreground ring-1 ring-border">
                    {activeDetail.catalog}
                  </span>
                  <span aria-hidden className="text-border">
                    /
                  </span>
                  <span className="rounded bg-background px-1.5 py-0.5 text-foreground ring-1 ring-border">
                    {activeDetail.schema}
                  </span>
                  <span aria-hidden className="text-border">
                    /
                  </span>
                  <span className="font-medium text-primary">
                    {activeDetail.table}
                  </span>
                </nav>
                <div className="space-y-1.5">
                  <SheetTitle className="text-lg font-semibold tracking-tight text-foreground">
                    {activeDetail.table}
                  </SheetTitle>
                  <SheetDescription className="text-sm leading-relaxed text-muted-foreground">
                    {activeDetail.description}
                  </SheetDescription>
                </div>
                <p className="font-mono text-[10px] text-muted-foreground/90">
                  {activeDetail.fqn}
                </p>
              </SheetHeader>

              <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
                <section className="space-y-2">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Governance
                  </h3>
                  <div className="flex flex-wrap gap-1.5">
                    <Badge
                      variant="outline"
                      className="border-primary/25 text-[11px] font-normal text-primary"
                    >
                      {activeDetail.zone}
                    </Badge>
                    <Badge className="border-0 bg-primary/10 text-[11px] font-normal text-primary hover:bg-primary/10">
                      {activeDetail.classification}
                    </Badge>
                    <Badge
                      variant="secondary"
                      className="text-[11px] font-normal"
                    >
                      {activeDetail.permission}
                    </Badge>
                    {activeDetail.tags.map((t) => (
                      <Badge
                        key={t}
                        variant="outline"
                        className="text-[11px] font-normal text-muted-foreground"
                      >
                        {t}
                      </Badge>
                    ))}
                  </div>
                </section>

                <Separator className="my-5" />

                <section>
                  <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Physical overview
                  </h3>
                  <div className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-3">
                    {[
                      { label: "Owner", value: activeDetail.owner },
                      {
                        label: "Last refreshed",
                        value: activeDetail.lastRefreshed,
                      },
                      { label: "Row estimate", value: activeDetail.rowEstimate },
                      { label: "Logical size", value: activeDetail.logicalSize },
                      { label: "Format", value: activeDetail.format },
                      {
                        label: "Columns",
                        value: String(activeDetail.columnCount),
                      },
                    ].map((item) => (
                      <div
                        key={item.label}
                        className="bg-card px-3 py-2.5 first:border-l-0"
                      >
                        <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                          {item.label}
                        </p>
                        <p className="mt-1 break-words text-sm font-medium leading-snug text-foreground">
                          {item.value}
                        </p>
                      </div>
                    ))}
                  </div>
                </section>

                <Separator className="my-5" />

                <section className="space-y-2">
                  <div className="flex items-baseline justify-between gap-2">
                    <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Schema
                    </h3>
                    <span className="text-[11px] text-muted-foreground">
                      {activeDetail.columns.length} of {activeDetail.columnCount}{" "}
                      shown
                    </span>
                  </div>
                  <div className="overflow-hidden rounded-lg border border-border">
                    <div className="max-h-[min(42vh,22rem)] overflow-auto">
                      <Table>
                        <TableHeader>
                          <TableRow className="border-b border-border hover:bg-transparent [&>th]:bg-muted/60">
                            <TableHead className="sticky top-0 z-[1] h-9 min-w-[100px] pl-3 text-[11px] font-semibold">
                              Column
                            </TableHead>
                            <TableHead className="sticky top-0 z-[1] h-9 min-w-[88px] text-[11px] font-semibold">
                              Type
                            </TableHead>
                            <TableHead className="sticky top-0 z-[1] hidden h-9 w-16 text-[11px] font-semibold sm:table-cell">
                              Null
                            </TableHead>
                            <TableHead className="sticky top-0 z-[1] h-9 min-w-[120px] pr-3 text-[11px] font-semibold">
                              Description
                            </TableHead>
                          </TableRow>
                        </TableHeader>
                        <TableBody>
                          {activeDetail.columns.map((col, i) => (
                            <TableRow
                              key={col.name}
                              className={cn(
                                "border-border/80 hover:bg-muted/40",
                                i % 2 === 1 && "bg-muted/20"
                              )}
                            >
                              <TableCell className="py-2 pl-3 font-mono text-xs font-medium text-primary">
                                {col.name}
                              </TableCell>
                              <TableCell className="py-2 font-mono text-[11px] text-muted-foreground">
                                {col.type}
                              </TableCell>
                              <TableCell className="hidden py-2 text-xs text-muted-foreground sm:table-cell">
                                {col.nullable}
                              </TableCell>
                              <TableCell className="max-w-[200px] py-2 pr-3 text-xs leading-snug text-muted-foreground">
                                {col.description}
                              </TableCell>
                            </TableRow>
                          ))}
                        </TableBody>
                      </Table>
                    </div>
                  </div>
                </section>
              </div>

              <SheetFooter className="shrink-0 gap-2 border-t border-border bg-card/80 px-5 py-4 backdrop-blur-sm sm:flex-row sm:justify-end">
                {activeDetail.tableDetailHref && (
                  <Button
                    className="w-full gap-2 sm:w-auto"
                    render={
                      <Link href={activeDetail.tableDetailHref} />
                    }
                    onClick={() => setDetailOpen(false)}
                  >
                    Open table page
                    <ExternalLink className="size-4" />
                  </Button>
                )}
                <Button
                  variant="outline"
                  className="w-full gap-2 sm:w-auto"
                  render={<Link href="/query-studio" />}
                  onClick={() => setDetailOpen(false)}
                >
                  Query in Query Studio
                </Button>
              </SheetFooter>
            </div>
          )}
        </SheetContent>
      </Sheet>
    </div>
  )
}
