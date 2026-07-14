"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import {
  Server,
  Award,
  Package,
  HardDrive,
  Database,
  Table2,
  HardDriveIcon,
  History,
  ArrowRight,
  Search,
} from "lucide-react"
import { Card, CardContent, CardFooter, CardHeader } from "@/components/ui/card"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination"
import { cn } from "@/lib/utils"

const LAYERS = ["Raw", "Bronze", "Silver", "Gold", "Semantic"] as const
type Layer = (typeof LAYERS)[number]

const SUMMARY_DATA: Record<
  Layer,
  { label: string; tables: number; storage: string; icon: React.ElementType; iconBg: string }
> = {
  Raw: {
    label: "Raw Ingested Data",
    tables: 142,
    storage: "15 TB",
    icon: Server,
    iconBg: "bg-primary/15",
  },
  Bronze: {
    label: "Bronze",
    tables: 89,
    storage: "12 TB",
    icon: Award,
    iconBg: "bg-primary/20",
  },
  Silver: {
    label: "Silver",
    tables: 45,
    storage: "10 TB",
    icon: Package,
    iconBg: "bg-primary/10",
  },
  Gold: {
    label: "Gold",
    tables: 45,
    storage: "10 TB",
    icon: HardDrive,
    iconBg: "bg-primary/15",
  },
  Semantic: {
    label: "Semantic",
    tables: 45,
    storage: "10 TB",
    icon: Database,
    iconBg: "bg-primary/20",
  },
}

interface TableItem {
  id: string
  name: string
  layer: Layer
  tags: string[]
  rows: string
  size: string
  updated: string
}

const SAMPLE_TABLES: TableItem[] = [
  {
    id: "1",
    name: "Customer_revenue",
    layer: "Raw",
    tags: ["Customer", "Finance", "Revenue"],
    rows: "2.8 M Rows",
    size: "1.2 GB",
    updated: "1 Hours Ago",
  },
  {
    id: "2",
    name: "Customer_revenue",
    layer: "Raw",
    tags: ["Customer", "Finance", "Revenue"],
    rows: "2.8 M Rows",
    size: "1.2 GB",
    updated: "1 Hours Ago",
  },
  {
    id: "3",
    name: "Customer_revenue",
    layer: "Raw",
    tags: ["Customer", "Finance", "Revenue"],
    rows: "2.8 M Rows",
    size: "1.2 GB",
    updated: "1 Hours Ago",
  },
  {
    id: "4",
    name: "Sales_analytics",
    layer: "Bronze",
    tags: ["Sales", "Analytics"],
    rows: "1.5 M Rows",
    size: "800 MB",
    updated: "2 Hours Ago",
  },
  {
    id: "5",
    name: "Inventory_snapshot",
    layer: "Bronze",
    tags: ["Inventory", "Operations"],
    rows: "500 K Rows",
    size: "300 MB",
    updated: "5 Hours Ago",
  },
  {
    id: "6",
    name: "Orders_fact",
    layer: "Raw",
    tags: ["Orders", "Sales"],
    rows: "5.1 M Rows",
    size: "2.4 GB",
    updated: "30 Min Ago",
  },
  {
    id: "7",
    name: "Events_log",
    layer: "Raw",
    tags: ["Events", "Log"],
    rows: "12 M Rows",
    size: "4.8 GB",
    updated: "15 Min Ago",
  },
  {
    id: "8",
    name: "Products_staging",
    layer: "Raw",
    tags: ["Product", "Staging"],
    rows: "800 K Rows",
    size: "450 MB",
    updated: "3 Hours Ago",
  },
  {
    id: "9",
    name: "Users_activity",
    layer: "Raw",
    tags: ["Users", "Activity"],
    rows: "9.2 M Rows",
    size: "3.1 GB",
    updated: "45 Min Ago",
  },
]

const ITEMS_PER_PAGE = 6

/**
 * Landing dashboard at `/`.
 *
 * Layout:
 * 1. Five summary cards (one per lake layer: Raw / Bronze / Silver / Gold / Semantic),
 *    sourced from the static `SUMMARY_DATA` map.
 * 2. A table browser with a layer Tabs filter, a free-text search, paginated
 *    cards (6 per page via `ITEMS_PER_PAGE`), and per-card "Explore" link to
 *    `/tables/[id]`.
 *
 * State: `activeLayer`, `searchQuery`, `currentPage` (all client-side; the data
 * is the static `SAMPLE_TABLES` fixture).
 */
export default function DashboardPage() {
  const [activeLayer, setActiveLayer] = useState<Layer>("Raw")
  const [searchQuery, setSearchQuery] = useState("")
  const [currentPage, setCurrentPage] = useState(1)

  const filteredTables = useMemo(() => {
    return SAMPLE_TABLES.filter((t) => {
      const matchLayer = t.layer === activeLayer
      const matchSearch =
        !searchQuery.trim() ||
        t.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        t.tags.some((tag) => tag.toLowerCase().includes(searchQuery.toLowerCase()))
      return matchLayer && matchSearch
    })
  }, [activeLayer, searchQuery])

  const totalPages = Math.max(1, Math.ceil(filteredTables.length / ITEMS_PER_PAGE))
  const paginatedTables = useMemo(() => {
    const start = (currentPage - 1) * ITEMS_PER_PAGE
    return filteredTables.slice(start, start + ITEMS_PER_PAGE)
  }, [filteredTables, currentPage])

  /** Switches the active layer tab and resets pagination back to page 1. */
  const handleLayerChange = (v: string) => {
    setActiveLayer(v as Layer)
    setCurrentPage(1)
  }
  /** Updates the free-text search and resets pagination back to page 1. */
  const handleSearchChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setSearchQuery(e.target.value)
    setCurrentPage(1)
  }

  return (
    <div className="flex flex-col gap-4 p-4">
      {/* Header */}
      <header className="border-b border-border pb-2">
        <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
          Dashboard
        </h1>
        <p className="text-[14px] leading-none tracking-[-0.084px] text-muted-foreground">
          Overview of all your workloads and resources
        </p>
      </header>

      {/* Summary cards */}
      <section className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
        {LAYERS.map((layer) => {
          const data = SUMMARY_DATA[layer]
          const Icon = data.icon
          return (
            <Card
              key={layer}
              className="flex flex-1 flex-col gap-0 overflow-hidden rounded-lg border-border bg-card py-0 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.1),0px_1px_3px_0px_rgba(0,0,0,0.1)]"
            >
              <CardHeader className="flex flex-row items-center justify-between gap-2 pb-1.5 pt-4 px-4">
                <span className="text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
                  {data.label}
                </span>
                <div
                  className={cn(
                    "flex items-center justify-center rounded p-1",
                    data.iconBg
                  )}
                >
                  <Icon className="size-4 shrink-0 text-primary opacity-70" />
                </div>
              </CardHeader>
              <CardContent className="flex flex-col gap-1 pb-4 pt-0 px-4">
                <div className="flex items-end gap-1 font-medium leading-none">
                  <span className="text-[20px] tracking-[-0.12px] text-primary">
                    {data.tables}
                  </span>
                  <span className="text-base tracking-[-0.096px] text-muted-foreground">
                    Tables
                  </span>
                </div>
                <p className="text-xs leading-4 tracking-[-0.072px] text-muted-foreground">
                  {data.storage}
                </p>
              </CardContent>
            </Card>
          )
        })}
      </section>

      {/* List Tables section */}
      <section className="flex flex-col gap-3">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <h2 className="text-[20px] font-medium leading-7 tracking-[-0.12px] text-primary">
            List Tables
          </h2>
          <div className="flex items-center gap-3">
            <Tabs
              value={activeLayer}
              onValueChange={handleLayerChange}
              className="w-full max-w-[379px]"
            >
              <TabsList className="h-auto w-full gap-0 rounded-md bg-secondary p-1">
                {LAYERS.map((layer) => (
                  <TabsTrigger
                    key={layer}
                    value={layer}
                    className={cn(
                      "flex-1 rounded px-3 py-1.5 text-xs font-medium leading-4 tracking-[-0.072px]",
                      "data-active:bg-background data-active:text-primary data-active:shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
                      "data-inactive:text-muted-foreground"
                    )}
                  >
                    {layer}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            <div className="relative w-[234px]">
              <Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 shrink-0 text-muted-foreground" />
              <Input
                type="search"
                placeholder="Search"
                value={searchQuery}
                onChange={handleSearchChange}
                className="h-10 rounded-md border-border pl-9 pr-3 text-sm placeholder:text-muted-foreground"
              />
            </div>
          </div>
        </div>

        {/* Table cards grid */}
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          {filteredTables.length === 0 ? (
            <p className="col-span-full py-8 text-center text-sm text-muted-foreground">
              No tables match the current filter or search.
            </p>
          ) : (
            paginatedTables.map((table) => (
              <Card
                key={table.id}
                className="overflow-hidden rounded-lg border-border bg-muted/50 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.1),0px_1px_3px_0px_rgba(0,0,0,0.1)] dark:bg-muted/30"
              >
                <CardHeader className="gap-1.5 pb-3 pt-6 px-6">
                  <p className="text-base font-medium leading-none tracking-[-0.096px] text-primary">
                    {table.name}
                  </p>
                  <div className="flex flex-wrap gap-1.5">
                    {table.tags.map((tag) => (
                      <Badge
                        key={tag}
                        variant="outline"
                        className="rounded-md border-border px-2.5 py-0.5 text-xs font-semibold leading-4 tracking-[-0.072px] text-muted-foreground"
                      >
                        {tag}
                      </Badge>
                    ))}
                  </div>
                </CardHeader>
                <CardContent className="px-6 py-0">
                  <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-border bg-muted/50 p-3.5">
                    <div className="flex items-center gap-1 text-sm font-medium leading-5 tracking-[-0.084px] text-muted-foreground">
                      <Table2 className="size-3.5 shrink-0 text-primary" />
                      {table.rows}
                    </div>
                    <div className="flex items-center gap-1 text-sm font-medium leading-5 tracking-[-0.084px] text-muted-foreground">
                      <HardDriveIcon className="size-3.5 shrink-0 text-primary" />
                      {table.size}
                    </div>
                    <div className="flex items-center gap-1 text-sm font-medium leading-5 tracking-[-0.084px] text-muted-foreground">
                      <History className="size-3.5 shrink-0 text-primary" />
                      {table.updated}
                    </div>
                  </div>
                </CardContent>
                <CardFooter className="flex justify-end border-t-0 bg-transparent pb-6 pt-6 px-6">
                  <Link
                    href={`/tables/${table.id}`}
                    className={cn(
                      "inline-flex h-9 items-center justify-center gap-2 rounded-md bg-primary px-4 text-sm font-medium leading-5 tracking-[-0.084px] text-primary-foreground transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    )}
                  >
                    Explore
                    <ArrowRight className="size-4" />
                  </Link>
                </CardFooter>
              </Card>
            ))
          )}

        {/* Pagination — border and primary styling */}
        {filteredTables.length > 0 && totalPages > 1 && (
          <Pagination className="mt-4 w-full justify-center">
            <PaginationContent className="flex gap-1 rounded-md border border-border bg-card p-1 shadow-sm">
              <PaginationItem>
                <PaginationPrevious
                  href="#"
                  onClick={(e) => {
                    e.preventDefault()
                    setCurrentPage((p) => Math.max(1, p - 1))
                  }}
                  className={cn(
                    "h-8 rounded-md border-0 text-muted-foreground hover:bg-muted hover:text-primary",
                    currentPage <= 1 && "pointer-events-none opacity-50"
                  )}
                />
              </PaginationItem>
              {Array.from({ length: totalPages }, (_, i) => i + 1).map((page) => (
                <PaginationItem key={page}>
                  <PaginationLink
                    href="#"
                    onClick={(e) => {
                      e.preventDefault()
                      setCurrentPage(page)
                    }}
                    isActive={currentPage === page}
                    className={cn(
                      "h-8 min-w-8 rounded-md text-sm font-medium",
                      "border-0 text-muted-foreground hover:bg-muted hover:text-primary",
                      "data-[active]:bg-primary data-[active]:text-primary-foreground data-[active]:hover:bg-primary data-[active]:hover:text-primary-foreground"
                    )}
                  >
                    {page}
                  </PaginationLink>
                </PaginationItem>
              ))}
              <PaginationItem>
                <PaginationNext
                  href="#"
                  onClick={(e) => {
                    e.preventDefault()
                    setCurrentPage((p) => Math.min(totalPages, p + 1))
                  }}
                  className={cn(
                    "h-8 rounded-md border-0 text-muted-foreground hover:bg-muted hover:text-primary",
                    currentPage >= totalPages && "pointer-events-none opacity-50"
                  )}
                />
              </PaginationItem>
            </PaginationContent>
          </Pagination>
        )}
        </div>
      </section>
    </div>
  )
}
