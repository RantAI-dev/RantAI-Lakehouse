"use client"

import { useState } from "react"
import { ArrowRight, GitBranch, Table2 } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

const TABLES = [
  { id: "gold.orders_fact", label: "gold.orders_fact" },
  { id: "silver.dim_customer", label: "silver.dim_customer" },
  { id: "semantic.support_embeddings", label: "semantic.support_embeddings" },
]

const TABLE_GRAPH: Record<
  string,
  { nodes: { id: string; title: string; layer: string }[]; edges: [string, string][] }
> = {
  "gold.orders_fact": {
    nodes: [
      { id: "raw", title: "raw.ingest.orders", layer: "Raw" },
      { id: "bronze", title: "bronze.orders_stg", layer: "Bronze" },
      { id: "silver", title: "silver.orders_clean", layer: "Silver" },
      { id: "gold", title: "gold.orders_fact", layer: "Gold" },
    ],
    edges: [
      ["raw", "bronze"],
      ["bronze", "silver"],
      ["silver", "gold"],
    ],
  },
  "silver.dim_customer": {
    nodes: [
      { id: "raw", title: "raw.crm_snapshot", layer: "Raw" },
      { id: "bronze", title: "bronze.customers", layer: "Bronze" },
      { id: "silver", title: "silver.dim_customer", layer: "Silver" },
    ],
    edges: [
      ["raw", "bronze"],
      ["bronze", "silver"],
    ],
  },
  "semantic.support_embeddings": {
    nodes: [
      { id: "gold", title: "gold.tickets_fact", layer: "Gold" },
      { id: "sem", title: "semantic.support_embeddings", layer: "Semantic" },
    ],
    edges: [["gold", "sem"]],
  },
}

const COLUMN_LINEAGE = [
  { col: "order_id", from: "silver.orders_clean.order_id", to: "gold.orders_fact.order_id" },
  { col: "amount", from: "silver.orders_clean.amount_local", to: "gold.orders_fact.amount" },
  { col: "customer_id", from: "silver.dim_customer.customer_id", to: "gold.orders_fact.customer_id" },
]

export default function LineagePage() {
  const [table, setTable] = useState(TABLES[0]!.id)

  const graph = TABLE_GRAPH[table] ?? TABLE_GRAPH["gold.orders_fact"]!

  return (
    <div className="flex flex-col gap-4">
      <header className="border-b border-border pb-2">
        <div className="flex flex-wrap items-center gap-2">
          <GitBranch className="size-7 text-primary" aria-hidden />
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Lineage
          </h1>
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          Table and column lineage across lake zones.
        </p>
      </header>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
        <div className="flex min-w-0 flex-col gap-1.5 sm:max-w-md">
          <Label className="text-primary">Focus table</Label>
          <Select value={table} onValueChange={(v) => setTable(v ?? TABLES[0]!.id)}>
            <SelectTrigger className="h-10 rounded-md border-border text-primary">
              <SelectValue placeholder="Select table" />
            </SelectTrigger>
            <SelectContent>
              {TABLES.map((t) => (
                <SelectItem key={t.id} value={t.id}>
                  {t.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <Tabs defaultValue="table" className="flex flex-col gap-4">
        <TabsList className="h-auto w-fit flex-wrap gap-1 rounded-md bg-secondary p-1">
          <TabsTrigger
            value="table"
            className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
          >
            Table lineage
          </TabsTrigger>
          <TabsTrigger
            value="column"
            className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
          >
            Column lineage
          </TabsTrigger>
        </TabsList>

        <TabsContent value="table" className="mt-0">
          <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="border-b border-border px-4 py-3">
              <h2 className="text-base font-medium text-primary">Upstream → target</h2>
              <p className="text-sm text-muted-foreground">
                Flow for {table}
              </p>
            </CardHeader>
            <CardContent className="flex flex-col gap-4 p-4">
              <div className="flex flex-wrap items-center justify-center gap-2 md:justify-start">
                {graph.nodes.map((n, i) => (
                  <div key={n.id} className="flex flex-wrap items-center gap-2">
                    {i > 0 && (
                      <ArrowRight className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                    )}
                    <div
                      className={cn(
                        "min-w-[140px] rounded-lg border border-border bg-muted/30 px-3 py-2 text-center shadow-sm",
                        n.title.includes("gold.") && "border-primary/40 bg-primary/5"
                      )}
                    >
                      <p className="font-mono text-[11px] font-medium leading-tight text-primary">
                        {n.title}
                      </p>
                      <Badge
                        variant="outline"
                        className="mt-1 border-primary/30 text-[10px] text-primary"
                      >
                        {n.layer}
                      </Badge>
                    </div>
                  </div>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                Edge list:{" "}
                {graph.edges.map(([a, b]) => `${a}→${b}`).join(", ")}
              </p>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="column" className="mt-0">
          <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="border-b border-border px-4 py-3">
              <div className="flex items-center gap-2">
                <Table2 className="size-4 text-primary" aria-hidden />
                <h2 className="text-base font-medium text-primary">
                  Column mappings
                </h2>
              </div>
              <p className="text-sm text-muted-foreground">
                Column mappings for gold.orders_fact.
              </p>
            </CardHeader>
            <CardContent className="grid gap-3 p-4 sm:grid-cols-1">
              {COLUMN_LINEAGE.map((row) => (
                <div
                  key={row.col}
                  className="flex flex-col gap-2 rounded-lg border border-border bg-muted/20 p-3 sm:flex-row sm:items-center sm:justify-between"
                >
                  <div className="font-mono text-sm font-medium text-primary">
                    {row.col}
                  </div>
                  <div className="flex flex-1 flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <span className="rounded border border-border bg-background px-2 py-1 font-mono">
                      {row.from}
                    </span>
                    <ArrowRight className="size-4 shrink-0" aria-hidden />
                    <span className="rounded border border-primary/30 bg-primary/5 px-2 py-1 font-mono text-primary">
                      {row.to}
                    </span>
                  </div>
                </div>
              ))}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}
