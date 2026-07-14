"use client"

import Link from "next/link"
import { useMemo, useState } from "react"
import { useParams } from "next/navigation"
import {
  ArrowLeft,
  Clock3,
  Loader2,
  Play,
  Sparkles,
  Users,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { SqlEditor } from "@/components/sql-editor"
import {
  getCollaborationProjectById,
  type Collaborator,
} from "@/lib/query-collaboration-fixtures"

const TAB_NL = "natural-language"
const TAB_SQL = "sql-editor"

const SAMPLE_SQL = `SELECT
  customer_id,
  SUM(amount) AS total_amount,
  COUNT(*) AS orders
FROM gold.orders_fact
WHERE order_date >= CURRENT_DATE - INTERVAL '30' DAY
GROUP BY 1
ORDER BY total_amount DESC
LIMIT 50;`

function statusDot(status: Collaborator["status"]) {
  if (status === "online") return "bg-emerald-500"
  if (status === "away") return "bg-amber-500"
  return "bg-slate-400"
}

export default function CollaborativeQueryStudioPage() {
  const params = useParams<{ projectId: string }>()
  const projectId = Array.isArray(params.projectId)
    ? params.projectId[0]
    : params.projectId

  const project = useMemo(
    () => (projectId ? getCollaborationProjectById(projectId) : null),
    [projectId]
  )

  const [tab, setTab] = useState(TAB_NL)
  const [nlPrompt, setNlPrompt] = useState("")
  const [nlRunning, setNlRunning] = useState(false)
  const [nlResult, setNlResult] = useState<string | null>(null)
  const [sqlQuery, setSqlQuery] = useState(SAMPLE_SQL)
  const [sqlRunning, setSqlRunning] = useState(false)
  const [sqlRows, setSqlRows] = useState<
    Array<{ customer_id: string; total_amount: string; orders: number }>
  >([])

  if (!project) {
    return (
      <Card>
        <CardHeader>
          <h1 className="text-lg font-semibold text-primary">Project not found</h1>
        </CardHeader>
        <CardContent>
          <Button variant="outline" render={<Link href="/query-studio/collaboration" />}>
            <ArrowLeft className="size-4" />
            Back to Project Management
          </Button>
        </CardContent>
      </Card>
    )
  }

  const runNl = () => {
    const q = nlPrompt.trim()
    if (!q || nlRunning) return
    setNlRunning(true)
    setNlResult(null)
    window.setTimeout(() => {
      setNlRunning(false)
      setNlResult(
        `Result preview for "${q}": 3 segments identified with strongest change in the last 30 days.`
      )
    }, 1500)
  }

  const runSql = () => {
    if (sqlRunning) return
    setSqlRunning(true)
    setSqlRows([])
    window.setTimeout(() => {
      setSqlRows([
        { customer_id: "C-1042", total_amount: "182340.20", orders: 53 },
        { customer_id: "C-552", total_amount: "149002.51", orders: 49 },
        { customer_id: "C-2201", total_amount: "121442.18", orders: 37 },
      ])
      setSqlRunning(false)
    }, 1300)
  }

  return (
    <div className="flex min-h-[min(720px,calc(100dvh-7rem))] flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border pb-2">
        <div>
          <Breadcrumb>
            <BreadcrumbList>
              <BreadcrumbItem>
                <BreadcrumbLink render={<Link href="/query-studio" />}>
                  Query Studio
                </BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem>
                <BreadcrumbLink render={<Link href="/query-studio/collaboration" />}>
                  Collaboration
                </BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem>
                <BreadcrumbPage>{project.name}</BreadcrumbPage>
              </BreadcrumbItem>
            </BreadcrumbList>
          </Breadcrumb>
          <h1 className="text-2xl font-semibold tracking-tight text-primary">
            Collaborative Query Studio
          </h1>
          <p className="text-sm text-muted-foreground">
            Project: {project.name} · Shared context for team querying.
          </p>
        </div>
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_360px]">
        <div className="space-y-4">
          <Card className="rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="border-b border-border">
              <h2 className="text-base font-medium text-primary">Query workspace</h2>
            </CardHeader>
            <CardContent className="p-4">
              <Tabs value={tab} onValueChange={setTab} className="space-y-4">
                <TabsList className="h-auto min-h-9 gap-1 rounded-md bg-secondary p-1">
                  <TabsTrigger value={TAB_NL} className="gap-2 rounded px-3 py-2 text-xs">
                    <Sparkles className="size-4" />
                    Natural language
                  </TabsTrigger>
                  <TabsTrigger value={TAB_SQL} className="gap-2 rounded px-3 py-2 text-xs">
                    SQL editor
                  </TabsTrigger>
                </TabsList>

                <TabsContent value={TAB_NL} className="space-y-3">
                  <Input
                    value={nlPrompt}
                    onChange={(e) => setNlPrompt(e.target.value)}
                    placeholder="Ask in natural language within this project context..."
                  />
                  <div className="flex justify-end">
                    <Button onClick={runNl} disabled={!nlPrompt.trim() || nlRunning}>
                      {nlRunning ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        <Play className="size-4" />
                      )}
                      Run NL Query
                    </Button>
                  </div>
                  {nlResult && (
                    <div className="rounded-md border border-border bg-muted/20 p-3 text-sm text-foreground">
                      {nlResult}
                    </div>
                  )}
                </TabsContent>

                <TabsContent value={TAB_SQL} className="space-y-3">
                  <SqlEditor value={sqlQuery} onChange={setSqlQuery} minHeight="230px" />
                  <div className="flex justify-end">
                    <Button onClick={runSql} disabled={sqlRunning}>
                      {sqlRunning ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        <Play className="size-4" />
                      )}
                      Run SQL
                    </Button>
                  </div>
                  <div className="overflow-x-auto rounded-md border border-border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>customer_id</TableHead>
                          <TableHead>total_amount</TableHead>
                          <TableHead>orders</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {sqlRows.length === 0 ? (
                          <TableRow>
                            <TableCell colSpan={3} className="text-center text-muted-foreground">
                              {sqlRunning ? "Running query..." : "Run SQL to see rows."}
                            </TableCell>
                          </TableRow>
                        ) : (
                          sqlRows.map((row) => (
                            <TableRow key={row.customer_id}>
                              <TableCell>{row.customer_id}</TableCell>
                              <TableCell>{row.total_amount}</TableCell>
                              <TableCell>{row.orders}</TableCell>
                            </TableRow>
                          ))
                        )}
                      </TableBody>
                    </Table>
                  </div>
                </TabsContent>
              </Tabs>
            </CardContent>
          </Card>
        </div>

        <div className="space-y-4">
          <Card className="rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="border-b border-border">
              <div className="flex items-center gap-2">
                <Users className="size-4 text-primary" />
                <h2 className="text-base font-medium text-primary">Collaboration panel</h2>
              </div>
            </CardHeader>
            <CardContent className="space-y-5 p-4">
              <section className="space-y-2">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Team members
                </p>
                <div className="space-y-2">
                  {project.members.map((member) => (
                    <div
                      key={member.id}
                      className="flex items-center justify-between rounded-md border border-border px-2.5 py-2"
                    >
                      <div className="flex items-center gap-2">
                        <span
                          className={`size-2 rounded-full ${statusDot(member.status)}`}
                          aria-hidden
                        />
                        <p className="text-sm text-foreground">{member.name}</p>
                      </div>
                      <Badge variant="outline">{member.role}</Badge>
                    </div>
                  ))}
                </div>
              </section>

              <section className="space-y-2">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Activity / Last update
                </p>
                <div className="space-y-2">
                  {project.activities.map((item) => (
                    <div key={item.id} className="rounded-md border border-border px-2.5 py-2">
                      <p className="text-sm text-foreground">
                        <span className="font-medium">{item.actor}</span> {item.action}
                      </p>
                      <p className="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
                        <Clock3 className="size-3" />
                        {item.when}
                      </p>
                    </div>
                  ))}
                </div>
              </section>

              <section className="space-y-2">
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Team query history
                </p>
                <div className="space-y-2">
                  {project.queryHistory.map((item) => (
                    <div key={item.id} className="rounded-md border border-border px-2.5 py-2">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs text-muted-foreground">{item.actor}</p>
                        <Badge variant="outline">{item.status}</Badge>
                      </div>
                      <p className="mt-1 text-sm text-foreground">{item.queryPreview}</p>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {item.mode} · {item.when}
                      </p>
                    </div>
                  ))}
                </div>
              </section>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}
