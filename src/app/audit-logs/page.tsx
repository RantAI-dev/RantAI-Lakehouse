"use client"

import { useMemo, useState } from "react"
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  ListOrdered,
  Search,
  XCircle,
} from "lucide-react"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"

type Outcome = "success" | "warning" | "error"

interface AuditLogRow {
  id: string
  dateTime: string
  event: string
  detail: string
  user: string
  ip: string
  action: string
  outcome: Outcome
}

const SAMPLE_AUDIT_LOGS: AuditLogRow[] = [
  {
    id: "1",
    dateTime: "2026-04-04 09:42:18",
    event: "Authentication",
    detail: "User signed in with SSO; session established in eu-west-1.",
    user: "aisha.rahman@company.com",
    ip: "203.0.113.44",
    action: "Sign in",
    outcome: "success",
  },
  {
    id: "2",
    dateTime: "2026-04-04 09:38:02",
    event: "Data export",
    detail: "Export job started for gold.orders_fact (Parquet, last 7 days).",
    user: "marcus.chen@company.com",
    ip: "198.51.100.12",
    action: "Export",
    outcome: "success",
  },
  {
    id: "3",
    dateTime: "2026-04-04 09:31:55",
    event: "Policy evaluation",
    detail: "Access to restricted classification blocked for requested table.",
    user: "elena.vasquez@company.com",
    ip: "192.0.2.201",
    action: "Query",
    outcome: "warning",
  },
  {
    id: "4",
    dateTime: "2026-04-04 09:15:40",
    event: "Pipeline run",
    detail: "Scheduled pipeline inventory_daily failed at bronze validation step.",
    user: "sa:pipeline-runner",
    ip: "10.0.4.18",
    action: "Run job",
    outcome: "error",
  },
  {
    id: "5",
    dateTime: "2026-04-04 08:58:11",
    event: "User management",
    detail: "Role updated from Viewer to Editor for james.okafor@company.com.",
    user: "david.park@company.com",
    ip: "203.0.113.90",
    action: "Update role",
    outcome: "success",
  },
  {
    id: "6",
    dateTime: "2026-04-04 08:44:33",
    event: "API request",
    detail: "Rate limit threshold reached for catalog search endpoint (burst).",
    user: "api-key:analytics-prod",
    ip: "198.51.100.77",
    action: "API call",
    outcome: "warning",
  },
  {
    id: "7",
    dateTime: "2026-04-04 08:22:09",
    event: "Storage",
    detail: "Lifecycle policy applied: 142 objects transitioned to cold tier.",
    user: "system",
    ip: "—",
    action: "Lifecycle",
    outcome: "success",
  },
  {
    id: "8",
    dateTime: "2026-04-04 07:55:00",
    event: "Authentication",
    detail: "MFA challenge failed after three attempts; account temporarily locked.",
    user: "unknown",
    ip: "192.0.2.88",
    action: "Sign in",
    outcome: "error",
  },
]

function toCsv(rows: AuditLogRow[]): string {
  const header =
    "date_time,event,detail,user,ip,action,outcome"
  const lines = rows.map(
    (r) =>
      `"${r.dateTime}","${r.event.replace(/"/g, '""')}","${r.detail.replace(/"/g, '""')}","${r.user}","${r.ip}","${r.action}","${r.outcome}"`
  )
  return [header, ...lines].join("\n")
}

export default function AuditLogsPage() {
  const [query, setQuery] = useState("")
  const [exporting, setExporting] = useState(false)

  const stats = useMemo(() => {
    const total = SAMPLE_AUDIT_LOGS.length
    const successful = SAMPLE_AUDIT_LOGS.filter((r) => r.outcome === "success").length
    const warnings = SAMPLE_AUDIT_LOGS.filter((r) => r.outcome === "warning").length
    const errors = SAMPLE_AUDIT_LOGS.filter((r) => r.outcome === "error").length
    return [
      { label: "Total events", value: total, icon: ListOrdered },
      { label: "Successful", value: successful, icon: CheckCircle2 },
      { label: "Warnings", value: warnings, icon: AlertTriangle },
      { label: "Errors", value: errors, icon: XCircle },
    ] as const
  }, [])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return SAMPLE_AUDIT_LOGS
    return SAMPLE_AUDIT_LOGS.filter(
      (r) =>
        r.event.toLowerCase().includes(q) ||
        r.detail.toLowerCase().includes(q) ||
        r.user.toLowerCase().includes(q) ||
        r.ip.toLowerCase().includes(q) ||
        r.action.toLowerCase().includes(q) ||
        r.dateTime.includes(q)
    )
  }, [query])

  return (
    <div className="flex flex-col gap-4">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
        <div>
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Audit Logs
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Immutable record of security and data events for compliance review.
          </p>
        </div>
        <Button
          variant="outline"
          size="default"
          type="button"
          disabled={exporting}
          className="h-10 gap-2 rounded-md border-border px-4 text-sm font-medium text-primary hover:bg-muted hover:text-primary"
          onClick={() => {
            setExporting(true)
            const csv = toCsv(filtered)
            const blob = new Blob([csv], { type: "text/csv;charset=utf-8" })
            const url = URL.createObjectURL(blob)
            const a = document.createElement("a")
            a.href = url
            a.download = `audit-logs-${new Date().toISOString().slice(0, 10)}.csv`
            a.rel = "noopener"
            a.click()
            URL.revokeObjectURL(url)
            window.setTimeout(() => setExporting(false), 800)
          }}
        >
          <Download className="size-4" />
          {exporting ? "Exported" : "Export"}
        </Button>
      </header>

      <section className="inline-flex w-fit max-w-full flex-wrap gap-2 self-start rounded-md bg-accent p-2">
        {stats.map((item) => {
          const Icon = item.icon
          return (
            <Card
              key={item.label}
              className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.1),0px_1px_3px_0px_rgba(0,0,0,0.1)]"
            >
              <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
                <div className="flex items-center gap-1.5">
                  <Icon className="size-3.5 shrink-0 text-muted-foreground" />
                  <p className="text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
                    {item.label}
                  </p>
                </div>
              </CardHeader>
              <CardContent className="px-3 pb-4 pt-1.5">
                <p className="text-[24px] font-medium leading-none tracking-[-0.144px] text-muted-foreground">
                  {item.value}
                </p>
              </CardContent>
            </Card>
          )
        })}
      </section>

      <section className="flex flex-col gap-3">
        <div className="relative max-w-md">
          <Search
            className="pointer-events-none absolute left-3 top-1/2 size-4 shrink-0 -translate-y-1/2 text-primary"
            aria-hidden
          />
          <Input
            type="search"
            placeholder="Search logs..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="h-10 w-full rounded-md border-border bg-background pl-9 pr-3 text-sm"
            aria-label="Search audit logs"
          />
        </div>

        <div className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-4">Date &amp; time</TableHead>
                <TableHead>Event</TableHead>
                <TableHead className="min-w-[200px]">Detail</TableHead>
                <TableHead>User</TableHead>
                <TableHead>IP</TableHead>
                <TableHead className="pr-4">Action</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className="py-10 text-center text-muted-foreground"
                  >
                    No log entries match your search.
                  </TableCell>
                </TableRow>
              ) : (
                filtered.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="whitespace-nowrap pl-4 font-mono text-xs text-muted-foreground">
                      {row.dateTime}
                    </TableCell>
                    <TableCell className="font-medium text-primary">
                      {row.event}
                    </TableCell>
                    <TableCell className="max-w-[320px] whitespace-normal text-muted-foreground">
                      {row.detail}
                    </TableCell>
                    <TableCell className="max-w-[200px] truncate text-sm text-muted-foreground">
                      {row.user}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {row.ip}
                    </TableCell>
                    <TableCell className="pr-4 font-medium text-primary">
                      {row.action}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </section>
    </div>
  )
}
