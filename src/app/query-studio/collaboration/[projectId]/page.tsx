"use client"

import Link from "next/link"
import { useCallback, useMemo, useRef, useState } from "react"
import { useParams } from "next/navigation"
import {
  ArrowLeft,
  Clock3,
  Loader2,
  Play,
  Sparkles,
  Terminal,
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
import { NaturalLanguageChat } from "@/components/query-studio/natural-language-chat"
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
  const [sqlQuery, setSqlQuery] = useState(SAMPLE_SQL)
  const sqlDirtyRef = useRef(false)
  const [pendingSqlLoad, setPendingSqlLoad] = useState<string | null>(null)
  const [sqlRunning, setSqlRunning] = useState(false)
  const [sqlRows, setSqlRows] = useState<
    Array<{ customer_id: string; total_amount: string; orders: number }>
  >([])

  const handleSqlChange = useCallback((v: string) => {
    sqlDirtyRef.current = true
    setSqlQuery(v)
  }, [])

  const applySqlLoad = useCallback((sql: string) => {
    setSqlQuery(sql)
    sqlDirtyRef.current = false
    setTab(TAB_SQL)
  }, [])

  /** Open in SQL Editor — never auto-runs; confirms before overwriting dirty edits. */
  const requestOpenInEditor = useCallback(
    (sql: string) => {
      const current = sqlQuery.trim()
      const hasUnsavedWork =
        sqlDirtyRef.current && current !== "" && current !== sql.trim()
      if (hasUnsavedWork) {
        setPendingSqlLoad(sql)
      } else {
        applySqlLoad(sql)
      }
    },
    [applySqlLoad, sqlQuery]
  )

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
        <div className="min-w-0 space-y-4">
          <Card className="rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="border-b border-border">
              <h2 className="text-base font-medium text-primary">Query workspace</h2>
            </CardHeader>
            <CardContent className="p-4">
              <Tabs value={tab} onValueChange={setTab} className="space-y-4">
                <TabsList className="h-auto min-h-9 gap-1 rounded-md bg-secondary p-1">
                  <TabsTrigger
                    value={TAB_NL}
                    className="gap-2 rounded px-3 py-2 text-xs data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
                  >
                    <Sparkles className="size-4" />
                    Natural language
                  </TabsTrigger>
                  <TabsTrigger
                    value={TAB_SQL}
                    className="gap-2 rounded px-3 py-2 text-xs data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
                  >
                    <Terminal className="size-4" />
                    SQL editor
                  </TabsTrigger>
                </TabsList>

                <TabsContent
                  value={TAB_NL}
                  className="mt-0 flex min-h-[min(560px,calc(100dvh-22rem))] flex-col data-[state=inactive]:hidden"
                >
                  <NaturalLanguageChat
                    onOpenInEditor={requestOpenInEditor}
                    emptyDescription={`Describe the data you need in plain language for “${project.name}”. The AI drafts the SQL — run it here, or open it in the SQL editor so the team can refine it together.`}
                  />
                </TabsContent>

                <TabsContent
                  value={TAB_SQL}
                  className="mt-0 space-y-3 data-[state=inactive]:hidden"
                >
                  <SqlEditor
                    value={sqlQuery}
                    onChange={handleSqlChange}
                    minHeight="230px"
                  />
                  <div className="flex justify-end">
                    <Button onClick={runSql} disabled={sqlRunning || !sqlQuery.trim()}>
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
                            <TableCell
                              colSpan={3}
                              className="text-center text-muted-foreground"
                            >
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

      <Dialog
        open={pendingSqlLoad !== null}
        onOpenChange={(open) => {
          if (!open) setPendingSqlLoad(null)
        }}
      >
        <DialogContent className="sm:max-w-md" showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>Replace the current SQL?</DialogTitle>
            <DialogDescription>
              The SQL editor has unsaved changes. Opening this query will
              overwrite what you&apos;re working on.
            </DialogDescription>
          </DialogHeader>
          <div className="px-5 py-4">
            <pre className="max-h-36 overflow-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-muted-foreground">
              {pendingSqlLoad ?? ""}
            </pre>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPendingSqlLoad(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => {
                if (pendingSqlLoad !== null) applySqlLoad(pendingSqlLoad)
                setPendingSqlLoad(null)
              }}
            >
              Replace query
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
