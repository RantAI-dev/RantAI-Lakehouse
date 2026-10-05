"use client"

import { useCallback, useState } from "react"
import Link from "next/link"
import { usePathname, useRouter, useSearchParams } from "next/navigation"
import { EntityHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { connectorService } from "@/services"
import { ConnectorDeleteDialog } from "./connector-delete-dialog"
import { ConnectorIngestPanel } from "./connector-ingest-panel"
import { ConnectorOverview } from "./connector-overview"
import { ConnectorProbeHistoryPanel } from "./connector-probe-history-panel"
import { DIRECTION_LABEL } from "./connectors-columns"

type ConnectorTab = "overview" | "ingest" | "tests"

/** The three tabs, in order. `?tab=` names any but the first. */
const TABS: { value: ConnectorTab; label: string }[] = [
  { value: "overview", label: "Overview" },
  { value: "ingest", label: "Ingest" },
  { value: "tests", label: "Connection tests" },
]

function parseTab(raw: string | null): ConnectorTab {
  return TABS.find((t) => t.value === raw)?.value ?? "overview"
}

/** The way back, in the same place on every state of the page. */
function SourcesLink() {
  return (
    <div className="flex items-center gap-2 text-xs text-muted-foreground">
      <Link href="/connectors" className="hover:underline">
        Sources
      </Link>
    </div>
  )
}

/**
 * One connector, at `/connectors/<id>`: what the Sources list used to open in
 * a side sheet, now an address of its own (plan
 * `docs/superpowers/plans/2026-10-05-connector-detail-page.md`, T1). The
 * header carries the status and the actions; below it, one tab each for the
 * overview, what the connector ingests (kept mounted, so tables picked but not
 * yet saved survive a look at another tab) and its connection tests. The open
 * tab lives in `?tab=`, so a refresh or a shared link lands on it.
 */
export function ConnectorDetailPage({ connectorId: id }: { connectorId: string }) {
  const router = useRouter()
  const pathname = usePathname()
  const searchParams = useSearchParams()
  // `keepDataOnReload`: a test reloads the detail to pick up the new health,
  // and on a whole page a skeleton in its place would also unmount the Ingest
  // tab and drop the table picks it is there to keep.
  const state = useService((s) => connectorService.getConnector(id, s), [id], {
    keepDataOnReload: true,
  })
  const [deleteOpen, setDeleteOpen] = useState(false)
  const urlTab = parseTab(searchParams.get("tab"))
  const [tab, setTab] = useState<ConnectorTab>(urlTab)
  // A link to another `?tab=` of this same page changes only the URL: the
  // page stays mounted, so the URL's tab is taken over here, during render.
  const [seenUrlTab, setSeenUrlTab] = useState(urlTab)
  if (urlTab !== seenUrlTab) {
    setSeenUrlTab(urlTab)
    setTab(urlTab)
  }
  const testAction = useServiceAction(
    withNotify(
      { success: "Connection test passed", error: "Connection test failed" },
      (signal, connectorId: string) =>
        connectorService.testConnection(connectorId, signal)
    )
  )
  const [historyKey, setHistoryKey] = useState(0)

  const selectTab = useCallback(
    (next: ConnectorTab) => {
      setTab(next)
      const params = new URLSearchParams(searchParams.toString())
      if (next === "overview") params.delete("tab")
      else params.set("tab", next)
      const query = params.toString()
      // The History API rather than the router: Next keeps
      // `useSearchParams` in step with it, and a tab switch needs no
      // server round trip.
      window.history.replaceState(null, "", query ? `${pathname}?${query}` : pathname)
    },
    [pathname, searchParams]
  )

  if (state.status === "loading") {
    return (
      <div className="flex flex-col gap-4">
        <SourcesLink />
        <LoadingSkeleton rows={4} />
      </div>
    )
  }
  if (state.status === "error") {
    // A connector that does not exist, or belongs to another tenant, is the
    // API's own sentence in the shared error state; the link keeps it from
    // being a dead end.
    return (
      <div className="flex flex-col gap-4">
        <SourcesLink />
        <ErrorState error={state.error} onRetry={state.reload} />
      </div>
    )
  }
  const c = state.data
  const inUse = c.dependentPipelines.length

  return (
    <div className="flex flex-col gap-4">
      <EntityHeader
        eyebrow={
          <Link href="/connectors" className="hover:underline">
            Sources
          </Link>
        }
        title={c.name}
        titleAccessory={
          <>
            <HealthBadge health={c.health} />
            <Pill tone="neutral">{DIRECTION_LABEL[c.direction]}</Pill>
            {c.environment ? <Pill tone="neutral">{c.environment}</Pill> : null}
          </>
        }
        description={c.type}
        actions={
          <>
            <Button
              size="sm"
              variant="outline"
              disabled={testAction.status === "pending"}
              onClick={async () => {
                await testAction.run(id)
                state.reload()
                setHistoryKey((k) => k + 1)
              }}
            >
              {testAction.status === "pending" ? "Testing…" : "Test connection"}
            </Button>
            <Button size="sm" variant="outline" render={<Link href={`/connectors/${id}/edit`} />}>
              Edit
            </Button>
            <Button size="sm" render={<Link href={`/pipelines/create?connectorId=${id}`} />}>
              Create pipeline
            </Button>
            {c.auditEventId ? (
              <Button
                size="sm"
                variant="ghost"
                render={<Link href={`/audit?event=${c.auditEventId}`} />}
              >
                Audit
              </Button>
            ) : null}
            <Button
              size="sm"
              variant="ghost"
              className="text-destructive hover:text-destructive"
              disabled={inUse > 0}
              title={
                inUse > 0
                  ? `Used by ${inUse} pipeline${inUse === 1 ? "" : "s"} (see Used by); those must be deleted or moved first`
                  : undefined
              }
              onClick={() => setDeleteOpen(true)}
            >
              Delete
            </Button>
          </>
        }
      />
      <ConnectorDeleteDialog
        connector={c}
        dependents={c.dependentPipelines}
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        onDeleted={() => router.push("/connectors")}
      />
      {testAction.data ? (
        <p
          className={
            !testAction.data.supported
              ? "text-sm text-muted-foreground"
              : testAction.data.ok
                ? "text-sm text-emerald-600 dark:text-emerald-400"
                : "text-sm text-destructive"
          }
        >
          {testAction.data.supported ? (
            <>
              {testAction.data.message}
              {testAction.data.latencyMs !== null ? ` · ${testAction.data.latencyMs} ms` : ""}
            </>
          ) : (
            <>Not testable · {testAction.data.message}</>
          )}
        </p>
      ) : null}
      <Tabs value={tab} onValueChange={(v) => selectTab(v as ConnectorTab)} className="gap-4">
        <TabsList>
          {TABS.map(({ value, label }) => (
            <TabsTrigger key={value} value={value}>
              {label}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="overview">
          {/* Remounted on every visit (and after a test), so it never
              shows a schedule or a run from before a change elsewhere. */}
          <ConnectorOverview key={historyKey} detail={c} onOpenTab={selectTab} />
        </TabsContent>
        <TabsContent value="ingest" keepMounted>
          <ConnectorIngestPanel connectorId={id} connectorName={c.name} />
        </TabsContent>
        <TabsContent value="tests">
          <ConnectorProbeHistoryPanel connectorId={id} refreshKey={historyKey} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
