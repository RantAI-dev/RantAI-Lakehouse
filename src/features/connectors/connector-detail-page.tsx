"use client"

import { useCallback, useRef, useState } from "react"
import Link from "next/link"
import { usePathname, useRouter, useSearchParams } from "next/navigation"
import {
  CircleCheckIcon,
  CircleXIcon,
  DatabaseIcon,
  InfoIcon,
  LayoutGridIcon,
  PlugZapIcon,
  type LucideIcon,
} from "lucide-react"
import { keepStripInView, LineTabsList, type LineTab } from "@/components/patterns/line-tabs"
import { MetadataList, type MetadataItem } from "@/components/patterns/metadata-list"
import { EntityHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useService, useServiceAction } from "@/hooks/use-service"
import { TEST_OUTCOME_TOAST, testOutcome, type TestOutcome } from "@/lib/connectors/test-result"
import { formatDateTime, formatRelativeTime } from "@/lib/format"
import { notifyError, notifyFailure, notifyInfo, notifySuccess } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { connectorService } from "@/services"
import type { ConnectorDetail, ConnectorTestResult } from "@/services/contracts/connectors"
import { ConnectorDeleteDialog } from "./connector-delete-dialog"
import { ConnectorIngestPanel } from "./connector-ingest-panel"
import { ConnectorOverview } from "./connector-overview"
import { ConnectorProbeHistoryPanel } from "./connector-probe-history-panel"
import { DIRECTION_LABEL } from "./connectors-columns"

type ConnectorTab = "overview" | "ingest" | "tests"

/** The three tabs, in order. `?tab=` names any but the first. */
const TABS: { value: ConnectorTab; label: string; icon: LucideIcon }[] = [
  { value: "overview", label: "Overview", icon: LayoutGridIcon },
  { value: "ingest", label: "Ingest", icon: DatabaseIcon },
  { value: "tests", label: "Connection tests", icon: PlugZapIcon },
]

function parseTab(raw: string | null): ConnectorTab {
  return TABS.find((t) => t.value === raw)?.value ?? "overview"
}

/**
 * The row of facts under the title, the asset page's `MetadataList` with the
 * connector's own: who it belongs to, how its credential is held and when it
 * was last tested. The type (the description) and the environment (a pill by
 * the name) are already in the header and are not repeated here (reviewer
 * SHOULD-FIX R1).
 */
function connectorFacts(c: ConnectorDetail): MetadataItem[] {
  return [
    { label: "Tenant", value: c.tenant || "Unassigned" },
    { label: "Residency", value: c.residency || "—" },
    { label: "Owner", value: c.owner || "—" },
    { label: "Credential", value: c.credentialManaged ? "Stored by lakehouse" : "Provisioned on the server" },
    {
      label: "Last test",
      value:
        c.lastTestAt === null ? (
          "Never tested"
        ) : (
          <span title={formatDateTime(c.lastTestAt)}>{formatRelativeTime(c.lastTestAt)}</span>
        ),
    },
  ]
}

/** How the notice under the header looks and what it calls each outcome. */
const NOTICE: Record<TestOutcome, { label: string; icon: LucideIcon; box: string; accent: string }> = {
  passed: {
    label: "Connection test passed",
    icon: CircleCheckIcon,
    box: "border-emerald-500/30 bg-emerald-500/5",
    accent: "text-emerald-600 dark:text-emerald-400",
  },
  failed: {
    label: "Connection test failed",
    icon: CircleXIcon,
    box: "border-destructive/30 bg-destructive/5",
    accent: "text-destructive",
  },
  unsupported: {
    label: "Not testable",
    icon: InfoIcon,
    box: "border-border bg-muted/30",
    accent: "text-muted-foreground",
  },
}

/**
 * What the last "Test connection" said, as a small bordered notice in the
 * tone of its outcome: the label (so the colour is never the only cue), then
 * the probe's message and, when one was measured, its latency.
 */
function TestNotice({ result }: { result: ConnectorTestResult }) {
  const outcome = testOutcome(result)
  const { label, icon: Icon, box, accent } = NOTICE[outcome]
  const latency = outcome !== "unsupported" && result.latencyMs !== null ? ` · ${result.latencyMs} ms` : ""
  return (
    <div role="status" className={cn("flex items-start gap-2 rounded-lg border px-3 py-2", box)}>
      <Icon className={cn("mt-0.5 size-4 shrink-0", accent)} aria-hidden />
      <p className="min-w-0 text-sm">
        <span className={cn("font-medium", accent)}>{label}</span>
        <span className="ml-2 break-words text-foreground">
          {result.message}
          {latency}
        </span>
      </p>
    </div>
  )
}

/**
 * The toast follows what the test found, not that the request went through:
 * `POST .../test` answers 200 for a probe that failed and for a type this
 * build cannot dial, so `withNotify` (which announces success for any answer)
 * is not used here.
 */
function announceTest(result: ConnectorTestResult) {
  const outcome = testOutcome(result)
  const title = TEST_OUTCOME_TOAST[outcome]
  if (outcome === "passed") notifySuccess(title)
  else if (outcome === "failed") notifyFailure(title)
  else notifyInfo(title)
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
 * header carries the status and the actions, with a row of facts under it
 * (plan section 10, U1-U2, U6); below that, one tab each for the overview, what
 * the connector ingests (kept mounted, so tables picked but not yet saved
 * survive a look at another tab) and its connection tests. The open tab lives
 * in `?tab=`, so a refresh or a shared link lands on it. The Ingest tab carries
 * the number of saved tables, and Connection tests a red `1` while the latest
 * test failed.
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
  const testAction = useServiceAction(async (signal, connectorId: string) => {
    try {
      const result = await connectorService.testConnection(connectorId, signal)
      // A cancelled request is not an answer; announce nothing for it.
      if (!signal.aborted) announceTest(result)
      return result
    } catch (err) {
      // The request itself failed (not a probe that failed): the shared
      // translation of the error, as `withNotify` would give it.
      notifyError(TEST_OUTCOME_TOAST.failed, err)
      throw err
    }
  })
  const [historyKey, setHistoryKey] = useState(0)
  // The latest test, for the red count on the Connection tests tab. Reloaded
  // after a test of this page; `keepDataOnReload` so the count does not blink
  // out while it is asked again.
  const latestProbe = useService((s) => connectorService.listProbeHistory(id, 1, s), [id], {
    keepDataOnReload: true,
  })
  // How many tables the Ingest tab's panel reports as saved. The panel is
  // mounted from the start (`keepMounted`) and already reads the spec, so the
  // count costs no request of its own.
  const [savedTables, setSavedTables] = useState<number | null>(null)
  const tabsRootRef = useRef<HTMLDivElement>(null)

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
      keepStripInView(tabsRootRef.current)
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
  const latest = latestProbe.data?.results[0]
  // A count only where there is something to say: the saved tables once the
  // panel has reported them, and a failing latest test. Never a placeholder.
  const counts: Record<ConnectorTab, LineTab["count"]> = {
    overview: null,
    ingest: savedTables === null ? null : { count: savedTables },
    tests: latest && !latest.ok ? { count: 1, tone: "danger" } : null,
  }
  const stripTabs: LineTab[] = TABS.map((t) => ({ ...t, count: counts[t.value] }))

  return (
    <div className="flex flex-col gap-3">
      <EntityHeader
        className="pb-3"
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
                latestProbe.reload()
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
      <MetadataList density="compact" columns={3} items={connectorFacts(c)} />
      <ConnectorDeleteDialog
        connector={c}
        dependents={c.dependentPipelines}
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        onDeleted={() => router.push("/connectors")}
      />
      {testAction.data ? <TestNotice result={testAction.data} /> : null}
      <div ref={tabsRootRef}>
        <Tabs value={tab} onValueChange={(v) => selectTab(v as ConnectorTab)} className="gap-3">
          <LineTabsList tabs={stripTabs} active={tab} />
          <TabsContent value="overview">
            {/* Remounted on every visit (and after a test), so it never
                shows a schedule or a run from before a change elsewhere. */}
            <ConnectorOverview key={historyKey} detail={c} onOpenTab={selectTab} />
          </TabsContent>
          <TabsContent value="ingest" keepMounted>
            <ConnectorIngestPanel
              connectorId={id}
              connectorName={c.name}
              layout="page"
              onTableCount={setSavedTables}
            />
          </TabsContent>
          <TabsContent value="tests">
            <ConnectorProbeHistoryPanel connectorId={id} refreshKey={historyKey} />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  )
}
