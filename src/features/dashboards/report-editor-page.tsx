"use client"

/**
 * Write or change a report as a document (`BI-62`, mockup D; spec targets T1
 * to T4): the title is the page's first line, text is written straight on
 * the page with no boxes or block labels, and charts are live.
 *
 * - `/` inserts (filtered as you type, ↑ ↓ Enter); Markdown shortcuts
 *   (`# `, `## `, `- `, `1. `, `> `, `---`) make blocks as you type.
 * - Selecting words shows bold, italic and link (⌘B, ⌘I, ⌘K).
 * - ⋮⋮ drags a block, + inserts under it, Alt+↑ ↓ moves it.
 * - Charts are picked from live pictures grouped by dashboard, at most 30
 *   (`BI-18` R4; the server refuses a 31st with its own sentence).
 * - The Charts tab of the one side panel (D6) holds every dashboard's charts
 *   as pictures to drag in or click in at the cursor.
 * - No Save button (D4): changes save about a second after typing stops and
 *   at once on blur, on leaving the page and when the tab is hidden
 *   (`src/lib/report-autosave.ts`); the header says Saving…, Saved, or
 *   Couldn't save with Retry, and the server's sentence when it refused. A
 *   new report is made on its first change that holds something, and the
 *   address becomes its edit address without a reload.
 *
 * The document is a view of the report's blocks (plan D3): it is made from
 * the blocks on load and turned back into blocks to save
 * (`src/lib/report-doc.ts`), so nothing new is stored. `id` absent: a new
 * report. Needs `dashboard:write`; without it the page says so
 * (`BI-18D-AC8`), and the server refuses the save anyway.
 */
import * as React from "react"
import Link from "next/link"
import type { Editor } from "@tiptap/core"
import { EditorContent, useEditor } from "@tiptap/react"
import { ArrowLeft, ChartColumn, Check, ChevronRight, CircleAlert, FileDown, Heading1, Heading2, List, ListOrdered, Loader2, Minus, TextQuote, Type } from "lucide-react"

import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useService } from "@/hooks/use-service"
import { useAuth } from "@/features/auth/auth-provider"
import { folderChoices, folderOptionText, labelledFolders } from "@/lib/dashboard-organise"
import { notifyError, notifySuccess } from "@/lib/notify"
import { Autosave, type AutosaveState } from "@/lib/report-autosave"
import { MAX_REPORT_CHARTS, blocksForSave } from "@/lib/report-blocks"
import { SLASH_ITEMS, blocksToDoc, docToBlocks, nearChartLimit, type DocNode, type SlashItemId } from "@/lib/report-doc"
import { cn } from "@/lib/utils"
import { dashboardReportService, dashboardService } from "@/services"
import type { ReportBlock, ReportView } from "@/services"
import { downloadReportPdf } from "./chart-download"
import { rememberRow } from "./dashboards-row"
import { FolderOptions } from "./folder-options"
import { ReportChartPicker } from "./report-chart-picker"
import { ChartShelfPortal, useReportChartsTab } from "./report-chart-shelf"
import { ReportChartsProvider, type ReportChartChoice } from "./report-charts"
import { ChartNode, ReportEditorUiContext, type PickerRequest, type ReportEditorUi } from "./report-editor-chart"
import { SlashStore, insertBlock, reportExtensions, type SlashItem } from "./report-editor-extensions"
import { BlockHandles, SelectionToolbar, SlashMenu, type LinkOpener, type SlashIcon } from "./report-editor-menus"
import { EASE, FOCUS, IconTip } from "./report-editor-ui"
import { REPORTS_TAB_HREF } from "./report-reader-page"
import { ReportingProvider } from "./reporting-context"

/** Select value for "no folder" (base-ui needs a non-empty value). */
const NO_FOLDER = "__none__"

const ICONS: Record<string, SlashIcon> = {
  text: Type,
  h1: Heading1,
  h2: Heading2,
  bullet: List,
  numbered: ListOrdered,
  quote: TextQuote,
  divider: Minus,
  chart: ChartColumn,
}

export function ReportEditorPage({ id }: { id?: string }) {
  const auth = useAuth()
  if (!auth.hasPermission("dashboard:write")) {
    return (
      <EmptyState
        title="You cannot write reports"
        description="Writing a report needs the dashboard write permission. Ask an administrator."
        action={
          <Button variant="outline" size="sm" render={<Link href={REPORTS_TAB_HREF} />}>
            <ArrowLeft data-icon="inline-start" />
            All reports
          </Button>
        }
      />
    )
  }
  return id ? <ExistingReportEditor id={id} /> : <ReportEditorFrame view={null} />
}

function ExistingReportEditor({ id }: { id: string }) {
  const state = useService((s) => dashboardReportService.getReport(id, s), [id])
  if (state.status === "loading") return <DocSkeleton />
  if (state.status === "error") {
    if (state.error.code === "not_found") {
      return <EmptyState title="This report is not available" description="It may have been deleted, or it is in a folder you cannot open." />
    }
    return <ErrorState error={state.error} onRetry={state.reload} />
  }
  return <ReportEditorFrame view={state.data} />
}

function ReportEditorFrame({ view }: { view: ReportView | null }) {
  return (
    <ReportChartsProvider seed={view?.charts}>
      <ReportingProvider reporting={view?.reporting ?? undefined}>
        <ReportEditor view={view} />
      </ReportingProvider>
    </ReportChartsProvider>
  )
}

/** The title, the chart count and the charts in use, read from the document after each change. */
type DocInfo = { title: string; count: number; used: ReadonlySet<string> }

function docInfo(editor: Editor): DocInfo {
  const used = new Set<string>()
  let count = 0
  editor.state.doc.forEach((n) => {
    if (n.type.name === "chart") {
      count += 1
      if (typeof n.attrs.chart === "string") used.add(n.attrs.chart)
    }
  })
  return { title: editor.state.doc.firstChild?.textContent ?? "", count, used }
}

/**
 * What the plugins read after render (the chart count for `/`, the insert a
 * dragged chart makes): a box with setters, so plugins made once still see
 * the latest values.
 */
class LiveBox {
  count = 0
  setCount(n: number) {
    this.count = n
  }
  private insert: (id: string, pos?: number) => void = () => {}
  setInsert(fn: (id: string, pos?: number) => void) {
    this.insert = fn
  }
  insertChart(id: string, pos?: number) {
    this.insert(id, pos)
  }
}

export type ReportSaveInput = { title: string; folderId: string; blocks: ReportBlock[] }

/** What a report with no title yet is saved as: the placeholder the page shows. */
export const UNTITLED_REPORT = "Untitled report"

function saveReport(input: ReportSaveInput, id: string | null) {
  const body = { ...input, title: input.title.trim() || UNTITLED_REPORT }
  return id ? dashboardReportService.updateReport(id, body) : dashboardReportService.createReport(body)
}

function ReportEditor({ view }: { view: ReportView | null }) {
  const auth = useAuth()
  const me = auth.user?.id
  const foldersState = useService((s) => dashboardService.listFolders(s), [])
  const folders = React.useMemo(() => folderChoices(labelledFolders(foldersState.data ?? [], me), me), [foldersState.data, me])
  const [folderId, setFolderId] = React.useState(view?.report.folderId || NO_FOLDER)
  const [info, setInfo] = React.useState<DocInfo>({ title: view?.report.title ?? "", count: 0, used: new Set() })
  const [picker, setPicker] = React.useState<PickerRequest | null>(null)
  const slash = React.useMemo(() => new SlashStore(), [])
  const live = React.useMemo(() => new LiveBox(), [])
  const linkRef = React.useRef<LinkOpener | null>(null)
  const area = React.useRef<HTMLDivElement>(null)
  const wrap = React.useRef<HTMLDivElement>(null)
  const reportId = view?.report.id

  // BI-61 T11: editing an existing report also counts as opening it for the Dashboards row.
  const openedTitle = view?.report.title
  React.useEffect(() => {
    if (reportId && openedTitle) rememberRow("report", reportId, openedTitle)
  }, [reportId, openedTitle])

  const items = React.useCallback((): SlashItem[] => {
    const run: Record<SlashItemId, SlashItem["run"]> = {
      text: (e, r) => e.chain().focus().deleteRange(r).setParagraph().run(),
      h1: (e, r) => e.chain().focus().deleteRange(r).setNode("heading", { level: 1 }).run(),
      h2: (e, r) => e.chain().focus().deleteRange(r).setNode("heading", { level: 2 }).run(),
      bullet: (e, r) => e.chain().focus().deleteRange(r).toggleBulletList().run(),
      numbered: (e, r) => e.chain().focus().deleteRange(r).toggleOrderedList().run(),
      quote: (e, r) => e.chain().focus().deleteRange(r).toggleBlockquote().run(),
      chart: (e, r) => {
        e.chain().focus().deleteRange(r).run()
        setPicker({ mode: "insert" })
      },
      divider: (e, r) => e.chain().focus().deleteRange(r).setHorizontalRule().run(),
      // "Ask AI to write…" joins the menu in PR B (plan D8).
      ai: () => {},
    }
    return SLASH_ITEMS.map((entry) =>
      entry.id === "chart"
        ? { ...entry, hint: `${live.count} of ${MAX_REPORT_CHARTS}`, disabled: live.count >= MAX_REPORT_CHARTS, run: run.chart }
        : { ...entry, run: run[entry.id] },
    )
  }, [live])

  // Made once for the page's life: the plugins read what changes through `live` and `linkRef`.
  const [extensions] = React.useState(() =>
    reportExtensions({
      chartNode: ChartNode,
      slash: { store: slash, items },
      onShelfDrop: (id, pos) => live.insertChart(id, pos),
      onLinkKey: () => linkRef.current?.open(),
    }),
  )

  // The document is made once from what was loaded; after that the editor owns it.
  const [initial] = React.useState(() => blocksToDoc(view?.report.title ?? "", view?.report.blocks ?? []))

  // D4: one autosave for the page's life. It starts from the page as loaded,
  // so opening a report (an old one with a link block included) saves nothing.
  const [autosave] = React.useState(() => {
    const loaded = docToBlocks(initial)
    return new Autosave<ReportSaveInput>({
      id: view?.report.id ?? null,
      initial: view ? { title: loaded.title, folderId: view.report.folderId, blocks: blocksForSave(loaded.blocks) } : undefined,
      isEmpty: (input) => !input.title.trim() && input.blocks.length === 0,
      save: saveReport,
      onCreated: (made) => {
        // The address becomes the edit address with no navigation, so the editor and its focus stay.
        window.history.replaceState(window.history.state, "", `/dashboards/reports/${encodeURIComponent(made)}/edit`)
      },
    })
  })
  const saveState = React.useSyncExternalStore(autosave.subscribe, autosave.get, autosave.get)
  // The editor's onUpdate is bound once; it reaches the newest snapshot through this.
  const changed = React.useRef<(() => void) | null>(null)

  const editor = useEditor(
    {
      immediatelyRender: false,
      shouldRerenderOnTransaction: false,
      extensions,
      content: initial,
      editorProps: { attributes: { class: "rd-doc", "aria-label": "Report", spellcheck: "true" } },
      onCreate: ({ editor: e }) => {
        const next = docInfo(e)
        live.setCount(next.count)
        setInfo(next)
        // A new report starts in its title.
        if (!reportId) e.commands.focus("start")
      },
      onUpdate: ({ editor: e }) => {
        const next = docInfo(e)
        live.setCount(next.count)
        setInfo(next)
        changed.current?.()
      },
      onBlur: () => {
        void autosave.flush()
      },
    },
    [extensions],
  )

  const insertChart = React.useCallback(
    (id: string, pos?: number) => {
      if (!editor || live.count >= MAX_REPORT_CHARTS) return
      insertBlock(editor, { type: "chart", attrs: { chart: id, width: "text" } }, pos)
    },
    [editor, live],
  )
  React.useEffect(() => {
    live.setInsert(insertChart)
  }, [live, insertChart])

  const pick = React.useCallback(
    (chart: ReportChartChoice) => {
      if (!editor || !picker) return
      if (picker.mode === "swap") {
        editor
          .chain()
          .command(({ tr }) => {
            const node = tr.doc.nodeAt(picker.pos)
            if (node?.type.name === "chart") tr.setNodeMarkup(picker.pos, undefined, { ...node.attrs, chart: chart.id })
            return true
          })
          .run()
      } else {
        insertChart(chart.id, picker.at)
      }
      setPicker(null)
      window.requestAnimationFrame(() => editor.commands.focus())
    },
    [editor, picker, insertChart],
  )

  /** What would be saved now. */
  const snapshot = React.useCallback((): ReportSaveInput | null => {
    if (!editor) return null
    const { title, blocks } = docToBlocks(editor.getJSON() as DocNode)
    return { title, folderId: folderId === NO_FOLDER ? "" : folderId, blocks: blocksForSave(blocks) }
  }, [editor, folderId])

  React.useEffect(() => {
    changed.current = () => {
      const input = snapshot()
      if (input) autosave.change(input)
    }
    // A folder change is a change too (a no-op when it is the loaded one).
    changed.current()
  }, [snapshot, autosave])

  // Leaving: a route change unmounts the page; hiding or closing the tab fires pagehide or
  // beforeunload. Each saves at once (best effort: a closing tab may cut the request), and
  // closing with something not yet saved asks the browser to confirm, so nothing is lost.
  React.useEffect(() => {
    const flush = () => {
      void autosave.flush()
    }
    const beforeUnload = (e: BeforeUnloadEvent) => {
      if (!autosave.pending) return
      void autosave.flush()
      e.preventDefault()
    }
    const hidden = () => {
      if (document.visibilityState === "hidden") flush()
    }
    window.addEventListener("pagehide", flush)
    window.addEventListener("beforeunload", beforeUnload)
    document.addEventListener("visibilitychange", hidden)
    return () => {
      window.removeEventListener("pagehide", flush)
      window.removeEventListener("beforeunload", beforeUnload)
      document.removeEventListener("visibilitychange", hidden)
      flush()
      autosave.dispose()
    }
  }, [autosave])

  const ui = React.useMemo<ReportEditorUi>(() => ({ readOnly: false, openPicker: setPicker }), [])
  // D6: Charts opens on a wide screen for an empty report, else as it was left.
  const chartsTab = useReportChartsTab({ ready: editor !== null, empty: !view || view.report.blocks.length === 0 })
  const pdf = Boolean(view?.pdfSupported && view.canDownload && reportId)
  const savedId = saveState.id
  const backHref = savedId ? `/dashboards/reports/${encodeURIComponent(savedId)}` : REPORTS_TAB_HREF

  return (
    <ReportEditorUiContext.Provider value={ui}>
      <div ref={area} className="rd-scroll relative flex min-h-full w-full flex-col">
        <EditorHeader
          title={info.title}
          backHref={backHref}
          folder={
            <Select value={folderId} onValueChange={(v) => setFolderId(String(v ?? NO_FOLDER))}>
              <SelectTrigger
                size="sm"
                aria-label="Folder"
                className="h-7 max-w-40 border-0 bg-transparent px-1.5 text-muted-foreground shadow-none hover:bg-muted hover:text-foreground dark:bg-transparent"
              >
                <SelectValue>{folderOptionText(folders, folderId === NO_FOLDER ? "" : folderId, "No folder")}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <FolderOptions folders={folders} noneValue={NO_FOLDER} noneLabel="No folder" />
              </SelectContent>
            </Select>
          }
          count={info.count}
          extra={<ToggleButton on={chartsTab.shown} icon={ChartColumn} label="Charts" onClick={chartsTab.toggle} />}
          pdf={pdf && reportId ? () => void downloadPdf(reportId) : undefined}
          status={<SaveStatus state={saveState} onRetry={() => void autosave.retry()} />}
        />
        <div className="px-0 pt-6 sm:px-10 sm:pt-10">
          <div ref={wrap} className="relative mx-auto w-full max-w-[720px]">
            {editor ? (
              <>
                <EditorContent editor={editor} />
                <SelectionToolbar editor={editor} linkRef={linkRef} />
                <BlockHandles editor={editor} area={area} wrap={wrap} />
              </>
            ) : (
              <DocLines />
            )}
          </div>
        </div>
        <SlashMenu store={slash} icons={ICONS} />
        <ChartShelfPortal host={chartsTab.host} shown={chartsTab.shown} used={info.used} count={info.count} onPick={(c) => insertChart(c.id)} />
        <ReportChartPicker
          open={picker !== null}
          onOpenChange={(o) => {
            if (!o) {
              setPicker(null)
              editor?.commands.focus()
            }
          }}
          swapping={picker?.mode === "swap"}
          current={picker?.mode === "swap" ? picker.current : undefined}
          used={info.used}
          count={info.count}
          onPick={pick}
        />
      </div>
    </ReportEditorUiContext.Provider>
  )
}

/**
 * The one status line (DESIGN.md §3): Saving…, Saved, or Couldn't save with
 * Retry, and the server's sentence beside it when it refused. On a phone the
 * words are for screen readers and the icon shows.
 */
function SaveStatus({ state, onRetry }: { readonly state: AutosaveState; readonly onRetry: () => void }) {
  if (state.status === "idle") return null
  if (state.status === "error") {
    return (
      <span className="inline-flex min-w-0 items-center gap-1.5 text-xs">
        {state.refusal ? (
          <span role="alert" className="min-w-0 truncate text-destructive max-sm:sr-only" title={state.refusal}>
            {state.refusal}
          </span>
        ) : null}
        <span role={state.refusal ? undefined : "alert"} className="inline-flex shrink-0 items-center gap-1 text-destructive">
          <CircleAlert className="size-3.5" aria-hidden />
          <span className="max-sm:sr-only">Couldn&apos;t save</span>
        </span>
        <span aria-hidden className="text-muted-foreground max-sm:hidden">·</span>
        <button type="button" onClick={onRetry} className={cn("shrink-0 rounded-sm font-medium text-primary hover:underline dark:text-sky-300", FOCUS)}>
          Retry
        </button>
      </span>
    )
  }
  const saving = state.status === "dirty" || state.status === "saving"
  return (
    <span aria-live="polite" className="inline-flex h-8 shrink-0 items-center gap-1.5 px-1 text-xs text-muted-foreground">
      {saving ? (
        <>
          <Loader2 className="size-3.5 animate-spin motion-reduce:animate-none" aria-hidden />
          <span className="max-sm:sr-only">Saving…</span>
        </>
      ) : (
        <>
          <Check className="size-3.5" aria-hidden />
          <span className="max-sm:sr-only">Saved</span>
        </>
      )}
    </span>
  )
}

async function downloadPdf(id: string) {
  try {
    const name = await downloadReportPdf(id, { paper: "a4", orientation: "portrait" })
    notifySuccess(`Downloaded ${name}`)
  } catch (err) {
    notifyError("Could not make the PDF", err)
  }
}

const ICON_BTN = cn("grid size-8 shrink-0 place-items-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground active:scale-[0.97]", EASE, FOCUS)

/** A toggle is never solid blue (DESIGN.md §3): an outline when off, a soft fill when on, the same width both ways. */
export function ToggleButton({
  on,
  onClick,
  icon: Icon,
  label,
}: {
  readonly on: boolean
  readonly onClick: () => void
  readonly icon: React.ComponentType<{ className?: string }>
  readonly label: string
}) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-sm font-medium ring-1 ring-inset active:scale-[0.97]",
        on ? "bg-primary/10 text-primary ring-transparent dark:bg-sky-400/15 dark:text-sky-300" : "text-primary ring-primary/30 hover:bg-primary/5 dark:text-sky-300 dark:ring-sky-300/30",
        EASE,
        FOCUS,
      )}
    >
      <Icon className="size-4" aria-hidden />
      <span className="max-sm:sr-only">{label}</span>
    </button>
  )
}

/**
 * The page's one row (DESIGN.md §3): back, where the report lives and its
 * title, then the save state, the chart count near the limit, the Charts toggle and
 * Download PDF. Sticky under the navbar, with a line once the page scrolls.
 */
function EditorHeader({
  title,
  backHref,
  folder,
  count,
  pdf,
  status,
  extra,
}: {
  readonly title: string
  readonly backHref: string
  readonly folder: React.ReactNode
  readonly count: number
  readonly pdf?: () => void
  readonly status: React.ReactNode
  readonly extra?: React.ReactNode
}) {
  const [scrolled, setScrolled] = React.useState(false)
  React.useEffect(() => {
    const on = () => setScrolled(window.scrollY > 4)
    on()
    window.addEventListener("scroll", on, { passive: true })
    return () => window.removeEventListener("scroll", on)
  }, [])
  return (
    <div
      className={cn(
        "sticky top-(--app-navbar-h) z-20 -mx-4 -mt-4 flex h-12 items-center gap-2 bg-background/90 px-2 backdrop-blur-md sm:-mx-5 sm:-mt-5 sm:px-3 lg:-mx-6 lg:-mt-6 lg:px-4",
        EASE,
        scrolled ? "shadow-[0_1px_0_var(--border)]" : "",
      )}
    >
      <IconTip label="Back">
        <Link href={backHref} aria-label="Back" className={ICON_BTN}>
          <ArrowLeft className="size-4" aria-hidden />
        </Link>
      </IconTip>
      <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground">
        <Link href={REPORTS_TAB_HREF} className={cn("hidden shrink-0 rounded-sm hover:text-foreground sm:inline", EASE, FOCUS)}>
          Reports
        </Link>
        <ChevronRight className="hidden size-3.5 shrink-0 sm:block" aria-hidden />
        <span className="hidden shrink-0 sm:inline">{folder}</span>
        <ChevronRight className="hidden size-3.5 shrink-0 sm:block" aria-hidden />
        <span className={cn("truncate", title ? "text-foreground" : "text-muted-foreground")} title={title || undefined}>
          {title || "Untitled report"}
        </span>
      </nav>
      <span className="ml-auto" />
      {status}
      {nearChartLimit(count) ? (
        <span
          data-testid="chart-count"
          className="hidden shrink-0 rounded-full bg-amber-500/14 px-2 text-xs leading-5 font-medium text-amber-800 tabular-nums sm:inline dark:text-amber-300"
        >
          {count} of {MAX_REPORT_CHARTS} charts
        </span>
      ) : null}
      {extra}
      {pdf ? (
        <IconTip label="Download PDF">
          <button type="button" aria-label="Download PDF" onClick={pdf} className={ICON_BTN}>
            <FileDown className="size-4" aria-hidden />
          </button>
        </IconTip>
      ) : null}
    </div>
  )
}

/** Before the editor mounts: the page's own shape (DESIGN.md §3, one loading look per page). */
function DocLines() {
  const bar = "animate-pulse rounded-md bg-muted motion-reduce:animate-none"
  return (
    <div aria-hidden className="grid gap-3 pt-2">
      <div className={cn(bar, "h-11 w-2/3")} />
      <div className={cn(bar, "mt-3 h-4 w-full")} />
      <div className={cn(bar, "h-4 w-11/12")} />
      <div className={cn(bar, "mt-6 h-7 w-1/3")} />
      <div className={cn(bar, "h-4 w-full")} />
      <div className={cn(bar, "mt-2 h-72 w-full rounded-2xl")} />
    </div>
  )
}

/** Loading an existing report: the header row and the same document lines. */
function DocSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading the report" className="flex w-full flex-col">
      <div className="h-12" />
      <div className="px-0 pt-6 sm:px-10 sm:pt-10">
        <div className="mx-auto w-full max-w-[720px]">
          <DocLines />
        </div>
      </div>
    </div>
  )
}
