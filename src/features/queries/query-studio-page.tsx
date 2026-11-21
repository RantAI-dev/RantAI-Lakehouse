"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { CircleStop, LoaderCircle, Play, Table2 } from "lucide-react"

import { ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot } from "@/features/copilot/use-copilot"
import { builtInBoardEditable } from "@/features/dashboards/chart-page"
import { useService } from "@/hooks/use-service"
import { useWindowWidth } from "@/hooks/use-window-width"
import { summarizeQuery } from "@/lib/page-context-summary"
import { queryDraftKey } from "@/lib/query-draft"
import { pinnedSnapshots, unpinSnapshots } from "@/lib/snapshot-picker"
import { formatSql } from "@/lib/sql-format"
import { queryEngineLabel } from "@/lib/status"
import { settingsService } from "@/services"
import { FormatSettingsDialog } from "./format-settings-dialog"
import { KeyboardShortcutsDialog } from "./keyboard-shortcuts-dialog"
import { NaturalLanguagePanel } from "./nl-panel"
import { QueryResultsSection } from "./query-results-section"
import { QueryResultTabs } from "./query-result-tabs"
import { QueryStudioTabs } from "./query-studio-tabs"
import { QueryTabBar } from "./query-tab-bar"
import { writeSaveQueryDraft, writeSaveSourceDraft } from "./save-draft"
import { ExplainCard, FixCard, FixWithAiButton } from "./sql-help-card"
import { SqlPanel } from "./sql-panel"
import { pickActions, studioActions, type PanelTab } from "./studio-actions"
import { StudioCommandMenu } from "./studio-command-menu"
import { StudioSidePanel } from "./studio-side-panel"
import {
  MoreActionsMenu,
  ModeTabs,
  PanelToggle,
  RunButton,
  SaveButton,
  useStudioKeys,
} from "./studio-toolbar"
import { TimeTravelChip, TimeTravelDialog } from "./time-travel-dialog"
import type { PinnedVersion } from "./time-travel-controls"
import { useFormatSettings } from "./use-format-settings"
import { useQueryStudio } from "./use-query-studio"

/** Where the side panel sits beside the editor rather than under the results: Tailwind's `lg`. */
const PANEL_BESIDE_WIDTH = 1024

/** Query Studio: SQL or a question in words, run against the lakehouse, with the plan before and the rows after. */
export function QueryStudioPage() {
  const { user, activeTenantId } = useAuth()
  // QA LH-QS-1-04 (QS-8): the draft belongs to one person in one tenant.
  const draftKey = user ? queryDraftKey(user.id, activeTenantId) : null
  // A different owner gets a new workspace. The first key is not a change:
  // the session may answer after the page has mounted.
  const [generation, setGeneration] = React.useState(0)
  const ownerRef = React.useRef<string | null>(null)
  React.useEffect(() => {
    if (draftKey === null) return
    if (ownerRef.current !== null && ownerRef.current !== draftKey) setGeneration((g) => g + 1)
    ownerRef.current = draftKey
  }, [draftKey])
  // `key` remounts the workspace, so the old one flushes its tabs under its
  // own key as it unmounts and the new one starts from its owner's entry.
  return <QueryStudioWorkspace key={generation} draftKey={draftKey} />
}

type StudioDialog = "format" | "shortcuts" | "time-travel" | null

/**
 * One owner's workspace (`UX-9`, layout A with mockup B's status bar): the
 * title row with the sections; the query tabs; one toolbar row (SQL | Ask,
 * the version pin, Save, ⋯, the side panel, Run); the editor with its
 * status bar; the result; and Schema | Plan in one panel on the right. The
 * command menu (⌘⇧P) lists every editor action from the same list the
 * menus read (`studio-actions.tsx`).
 *
 * The Saved and History cards that sat beside the editor are gone: the
 * sections are their pages, and their rows open a query here.
 */
function QueryStudioWorkspace({ draftKey }: { draftKey: string | null }) {
  // All the query state lives in the hook; this file is the layout and the
  // page's own controls (panel, dialogs, command menu).
  const studio = useQueryStudio(draftKey)
  const router = useRouter()
  const { hasPermission } = useAuth()
  const canAuthorSources = hasPermission("dashboard:sql")
  // Make a chart ends in the chart builder of the built-in dashboard (the
  // builder's own picker moves it to another), which needs the right to
  // edit that dashboard as well as the right to write the SQL source.
  const canMakeChart = canAuthorSources && builtInBoardEditable(hasPermission)
  const format = useFormatSettings()
  // UX-9 browser review: the result grid writes numbers in the deployment's
  // number locale, as the dashboards' tables do. Any signed-in person may
  // read it (`GET /api/settings/reporting`); until it answers, or if it
  // does not, the grid uses the server's own default.
  const reporting = useService((signal) => settingsService.getReporting(signal), [])
  const numberLocale = reporting.status === "success" ? reporting.data.numberLocale : undefined

  // The side panel is open on a laptop and closed on a narrow screen until
  // the person says otherwise; their choice then holds at every width.
  //
  // UX-9 review (DESIGN.md §3, one side panel at a time): Schema | Plan and
  // the AI Copilot panel share the right-hand slot. The Copilot panel is the
  // app shell's own state (`useCopilot`: docked right and expanded, which
  // `CopilotSidebar` draws and remembers in this browser); opening ours
  // collapses it, and opening it closes ours. Ours is not remembered across
  // visits, as before; Copilot's is, by the shell.
  const copilot = useCopilot()
  const copilotPanelOpen = copilot.dockPosition === "right" && copilot.expanded
  const width = useWindowWidth()
  const [panelChoice, setPanelChoice] = React.useState<boolean | null>(null)
  const panelOpen =
    panelChoice ?? (!copilotPanelOpen && (width === null || width >= PANEL_BESIDE_WIDTH))
  const [panelTab, setPanelTab] = React.useState<PanelTab>("schema")
  function openPanel() {
    setPanelChoice(true)
    if (copilotPanelOpen) copilot.setExpanded(false)
  }
  function showPanel(tab: PanelTab) {
    setPanelTab(tab)
    openPanel()
  }
  // Copilot's panel opened (its navbar button, the dock, its history menu):
  // ours makes room. A panel the person never chose closes by itself above.
  const copilotWasOpen = React.useRef(copilotPanelOpen)
  React.useEffect(() => {
    if (copilotPanelOpen && !copilotWasOpen.current) {
      setPanelChoice((choice) => (choice === true ? false : choice))
    }
    copilotWasOpen.current = copilotPanelOpen
  }, [copilotPanelOpen])
  const keys = useStudioKeys()

  const [dialog, setDialog] = React.useState<StudioDialog>(null)
  const [commandsOpen, setCommandsOpen] = React.useState(false)
  // The versions picked in this studio, by snapshot id: the chip names
  // their table and time. A pin that came in a link has no entry and the
  // chip names its version.
  const [picked, setPicked] = React.useState<Record<string, PinnedVersion>>({})
  const pinned = pinnedSnapshots(studio.sql).length > 0

  /**
   * `WEB-7` D5: saving is its own page now, so the statement it would save
   * is written to `sessionStorage` first (`save-draft.ts`) and the editor
   * travels with it. The editor keeps its text across the trip through its
   * own `localStorage` draft (`lib/query-draft.ts`), which is flushed when
   * this page unmounts (`useDebouncedCallback` flushes on unmount), so
   * Cancel or Save returns to the same SQL. `UX-9`: "Save as new" is the
   * same trip as a copy of the saved query the tab came from.
   */
  function openSavePage(mode: "update" | "create" = studio.saveTarget.mode) {
    writeSaveQueryDraft({
      sql: studio.targetStatement?.text ?? "",
      parameters: studio.saveParameters,
      mode,
      savedId: studio.saveTarget.savedId,
      tabId: studio.activeTabId,
    })
    router.push("/query-studio/save")
  }

  function openSaveSourcePage() {
    writeSaveSourceDraft({ sql: studio.targetStatement?.text ?? "" })
    router.push("/query-studio/save-source")
  }

  /**
   * `UX-9` Make a chart: the statement that made this result goes through
   * today's Save as SQL source page, which then opens the chart builder on
   * the source it saved (`next: "chart"`). No new API: the source is the
   * one `POST /api/dashboard/sources` writes, and the builder already takes
   * a source in its draft.
   */
  function makeChart(statement: string) {
    writeSaveSourceDraft({ sql: statement, next: "chart" })
    router.push("/query-studio/save-source")
  }

  const actions = studioActions({
    studio,
    onSave: () => openSavePage(),
    onSaveAsNew: () => openSavePage("create"),
    onSaveAsSource: canAuthorSources ? openSaveSourcePage : undefined,
    onFormat: () => studio.setSql(formatSql(studio.sql, format.settings)),
    onOpenFormatSettings: () => setDialog("format"),
    onOpenTimeTravel: () => setDialog("time-travel"),
    onUnpin: pinned ? () => studio.setSql(unpinSnapshots(studio.sql)) : undefined,
    onOpenShortcuts: () => setDialog("shortcuts"),
    panelOpen,
    onShowPanel: showPanel,
    onHidePanel: () => setPanelChoice(false),
    runKey: keys?.run,
  })
  const [save] = pickActions(actions, ["save"])
  const [estimateAgain] = pickActions(actions, ["estimate"])

  // Page-aware Copilot (plan §6): the SQL being written and the first rows
  // of its last result. Deferred so typing is not slowed by re-sending the
  // context on every keystroke.
  const { setPageContext } = copilot
  const deferredSql = React.useDeferredValue(studio.sql)
  // The rows on screen are the open statement's own: a statement that
  // never ran shows nothing, and the assistant reads the same rows the
  // person sees.
  const openRun = studio.activeRun
  const lastResult = openRun?.status === "done" ? openRun.result : null
  React.useEffect(() => {
    setPageContext({
      key: "query",
      title: "Write and run SQL",
      hint: "Ask about this query or its result, or have AI Copilot write SQL.",
      suggest: {
        ask: ["Explain this query", "What does this result show?", "Why might this query be slow?"],
        build: ["Turn this into a chart on a dashboard"],
      },
      system:
        "The user is in Query Studio. Prefer run_sql; offer to turn results into a chart.\n" +
        summarizeQuery(deferredSql, lastResult),
    })
    return () => setPageContext(null)
  }, [deferredSql, lastResult, setPageContext])

  const sqlMode = studio.tab === "sql"

  return (
    <div className="flex flex-col">
      {/* The app's one frame (`DESIGN.md` §3): the title left, the page's
          sections on the right, one 32 px row. */}
      <div className="flex min-h-8 items-center gap-2">
        <h1 className="mr-2 shrink-0 text-2xl leading-8 font-semibold tracking-[-0.02em] text-foreground">
          Query Studio
        </h1>
        <div aria-hidden className="flex-1" />
        <QueryStudioTabs />
      </div>
      {/* A `?saved=` link that names a query the caller may not see opens no
          tab: the studio says so instead of an empty editor (`QS-3` C5). */}
      {studio.savedLinkError ? (
        <p
          role="alert"
          className="mt-3 rounded-lg bg-destructive/8 px-3 py-2 text-sm text-destructive"
        >
          {studio.savedLinkError}
        </p>
      ) : null}
      {/* QS-12 (LH-QS-4-07), D15: a `?run=` link whose run is outside the
          caller's last 14 days opens no tab; say so where the saved-link
          error appears. */}
      {studio.runLinkError ? (
        <p
          role="alert"
          className="mt-3 rounded-lg bg-destructive/8 px-3 py-2 text-sm text-destructive"
        >
          {studio.runLinkError}
        </p>
      ) : null}

      {/* The workbench runs to the edges of the content area, so the panel's
          divider and the rows' dividers meet the frame (`app-frame.tsx`
          pads `<main>`; the strip undoes it, as `line-tabs.tsx` does). */}
      <div className="-mx-4 mt-4 flex flex-col border-t border-border sm:-mx-5 lg:-mx-6 lg:flex-row lg:items-start">
        <Tabs
          value={studio.tab}
          onValueChange={(v) => studio.setTab(v === "sql" ? "sql" : "nl")}
          className="min-w-0 flex-1 gap-0"
        >
          <div className="flex h-11 items-center border-b border-border px-3 sm:px-4">
            <QueryTabBar studio={studio} />
          </div>
          <div className="flex flex-wrap items-center gap-1.5 px-3 py-2 sm:px-4">
            <ModeTabs />
            <TimeTravelChip
              sql={studio.sql}
              known={picked}
              onOpen={() => setDialog("time-travel")}
              onRemove={() => studio.setSql(unpinSnapshots(studio.sql))}
            />
            <div aria-hidden className="flex-1" />
            {sqlMode && save ? (
              <SaveButton
                save={save}
                more={pickActions(actions, ["save-as-new", "save-source"])}
              />
            ) : null}
            <MoreActionsMenu
              groups={[
                pickActions(actions, ["format", "format-settings", "explain"]),
                pickActions(actions, ["time-travel", "unpin", "shortcuts"]),
              ]}
            />
            <PanelToggle
              open={panelOpen}
              onToggle={() => (panelOpen ? setPanelChoice(false) : openPanel())}
            />
            {sqlMode ? <RunButton studio={studio} /> : null}
          </div>
          <TabsContent value="sql" className="px-3 sm:px-4">
            <SqlPanel studio={studio} onOpenPlan={() => showPanel("plan")} />
          </TabsContent>
          <TabsContent value="nl" className="px-3 sm:px-4">
            <NaturalLanguagePanel studio={studio} />
          </TabsContent>

          <div className="mt-3 space-y-3 border-t border-border px-3 pt-3 pb-4 sm:px-4">
            {studio.explain.error ? (
              <ErrorState error={studio.explain.error} />
            ) : studio.explain.result !== null ? (
              <ExplainCard text={studio.explain.result} />
            ) : null}
            {studio.fix.error ? (
              <ErrorState error={studio.fix.error} />
            ) : studio.fix.result ? (
              <FixCard
                outcome={studio.fix.result}
                editorSql={studio.fix.statement?.text ?? studio.sql}
                onAccept={studio.fix.accept}
                onReject={studio.fix.reject}
              />
            ) : null}
            {/* The result of a run: one tab per statement, and the outcome of
                the statement that is open on it. */}
            {studio.activeRuns.length > 1 ? <QueryResultTabs studio={studio} /> : null}
            {openRun?.status === "done" ? (
              <QueryResultsSection
                result={openRun.result}
                runId={openRun.runId}
                downloadBusy={studio.download.busy}
                onDownload={(format) =>
                  studio.download.start(openRun.result.id, format, openRun.runId)
                }
                onMakeChart={canMakeChart ? () => makeChart(openRun.statement.text) : undefined}
                numberLocale={numberLocale}
              />
            ) : null}
            {openRun?.status === "error" ? (
              <div className="space-y-2">
                <ErrorState error={openRun.error} onRetry={() => void studio.runQuery()} />
                {/*
                 * QS-11 (LH-QS-1-04 F2), decision 13: Fix with AI is only for a
                 * statement the person can fix — the server's 400/422 that says
                 * something in the SQL is wrong. A permission refusal, a hidden
                 * table, an outage or a rate limit need a different action;
                 * changing the SQL cannot help. Fix with AI also works for
                 * ClickHouse only: the check that makes a proposal safe to show
                 * runs as EXPLAIN PLAN, which Trino lacks (plan 16 lifts this).
                 */}
                {openRun.error.code === "invalid_request" ? (
                  <div className="flex justify-center">
                    <FixWithAiButton
                      engine={studio.engine}
                      busy={studio.fix.busy}
                      onFix={() => void studio.fix.run(openRun.error.message, openRun.statement)}
                    />
                  </div>
                ) : null}
              </div>
            ) : null}
            {openRun?.status === "not-run" ? (
              <ResultNote
                icon={<CircleStop className="size-5" aria-hidden />}
                // QA LH-QS-4-10: a statement the person stopped is not one a failure skipped.
                text={
                  openRun.reason === "stopped"
                    ? "Stopped: you stopped the run before this statement finished."
                    : "Not run: the batch stopped at the failure before this statement."
                }
                action={
                  openRun.reason === "stopped" && !studio.running ? (
                    <Button variant="outline" onClick={() => void studio.runQuery()}>
                      <Play className="size-3.5" aria-hidden />
                      Run again
                    </Button>
                  ) : null
                }
              />
            ) : null}
            {openRun?.status === "pending" || openRun?.status === "running" ? (
              <RunningRows engine={studio.engine} />
            ) : null}
            {!openRun ? (
              <ResultNote
                icon={<Table2 className="size-5" aria-hidden />}
                title="Results show here"
                text={`Run a query${keys ? ` with ${keys.run}` : ""}, or open one you saved.`}
                action={
                  <Button variant="outline" render={<Link href="/query-studio/saved" />}>
                    Open a saved query
                  </Button>
                }
              />
            ) : null}
          </div>
        </Tabs>
        {panelOpen ? (
          <StudioSidePanel
            studio={studio}
            tab={panelTab}
            onTabChange={setPanelTab}
            estimateAgain={estimateAgain!}
            className="border-t border-border lg:sticky lg:top-16 lg:h-[calc(100svh-4rem-var(--content-bottom,1.5rem))] lg:w-[300px] lg:shrink-0 lg:border-t-0 lg:border-l"
          />
        ) : null}
      </div>

      <StudioCommandMenu open={commandsOpen} onOpenChange={setCommandsOpen} actions={actions} />
      <FormatSettingsDialog
        open={dialog === "format"}
        onOpenChange={(open) => setDialog(open ? "format" : null)}
        format={format}
      />
      <KeyboardShortcutsDialog
        open={dialog === "shortcuts"}
        onOpenChange={(open) => setDialog(open ? "shortcuts" : null)}
      />
      <TimeTravelDialog
        open={dialog === "time-travel"}
        onOpenChange={(open) => setDialog(open ? "time-travel" : null)}
        sql={studio.sql}
        engine={studio.engine}
        onApply={(next, version) => {
          setPicked((previous) => ({ ...previous, [version.snapshotId]: version }))
          studio.setSql(next)
        }}
      />
    </div>
  )
}

/** A quiet, centred line in the result area: no result yet, or a statement that did not run. */
function ResultNote({
  icon,
  title,
  text,
  action,
}: {
  readonly icon: React.ReactNode
  readonly title?: string
  readonly text: string
  readonly action?: React.ReactNode
}) {
  return (
    <div className="grid justify-items-center gap-1.5 px-6 py-10 text-center">
      <span className="grid size-10 place-items-center rounded-2xl bg-muted text-muted-foreground">
        {icon}
      </span>
      {title ? <p className="mt-1 text-sm font-semibold">{title}</p> : null}
      <p className="max-w-md text-[13px] leading-5 text-muted-foreground">{text}</p>
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  )
}

/** The result area while a statement runs: what is running, and rows still to come. */
function RunningRows({ engine }: { readonly engine: string }) {
  return (
    <div className="relative grid content-start" role="status">
      <div aria-hidden className="absolute inset-x-0 top-0 h-0.5 overflow-hidden rounded-full bg-primary/15">
        <div className="h-full w-1/4 animate-studio-running rounded-full bg-primary motion-reduce:animate-none" />
      </div>
      <p className="flex items-center gap-2 pt-3 pb-2 text-[13px] text-muted-foreground">
        <LoaderCircle className="size-3.5 animate-spin motion-reduce:animate-none" aria-hidden />
        Running on {queryEngineLabel(engine)}…
      </p>
      <div aria-hidden className="grid gap-px opacity-60">
        {Array.from({ length: 6 }, (_, i) => (
          <div key={i} className="flex gap-6 py-2">
            <span className="block h-3 w-40 animate-pulse rounded bg-muted motion-reduce:animate-none" />
            <span className="ml-auto block h-3 w-20 animate-pulse rounded bg-muted motion-reduce:animate-none" />
          </div>
        ))}
      </div>
    </div>
  )
}
