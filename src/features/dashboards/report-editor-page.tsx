"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useTheme } from "next-themes"
import {
  ArrowDown,
  ArrowLeft,
  ArrowUp,
  BarChart3,
  Heading,
  Link2,
  List,
  Pilcrow,
  PlusIcon,
  Trash2,
} from "lucide-react"

import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useAuth } from "@/features/auth/auth-provider"
import { folderChoices, folderOptionText, labelledFolders } from "@/lib/dashboard-organise"
import { withNotify } from "@/lib/notify"
import {
  MAX_REPORT_CHARTS,
  TOO_MANY_CHARTS,
  blocksForSave,
  canAddChart,
  chartCount,
  emptyBlock,
  insertBlock,
  listItems,
  listText,
  moveBlock,
  removeBlock,
  replaceBlock,
  safeReportUrl,
} from "@/lib/report-blocks"
import { cn } from "@/lib/utils"
import { dashboardReportService, dashboardService } from "@/services"
import type { ReportBlock, ReportChartEntry, ReportView } from "@/services"
import { FolderOptions } from "./folder-options"
import { ReportChart } from "./report-blocks"
import { ReportChartPicker, type PickedChart } from "./report-chart-picker"
import { REPORTS_TAB_HREF } from "./report-reader-page"
import { ReportingProvider } from "./reporting-context"

/** Select value for "no folder" (base-ui needs a non-empty value). */
const NO_FOLDER = "__none__"

/** A chart block the editor knows the title of, picked in this sitting. */
type Picked = Record<string, { title: string; boardName: string }>

/**
 * Write or change a report (`BI-18` part D, T5, decisions R1, R2 and R4): a
 * title, a folder, and blocks in order (heading, paragraph, list, link, and a
 * chart picked from a dashboard). Each block can be moved up or down or
 * removed. A chart already in the report is drawn live, as the reader will
 * see it; a chart added in this sitting shows its title until the report is
 * saved, because only the server reads a chart as a person. The thirtieth
 * chart is the last one the editor offers (R4); the server refuses a 31st with
 * its own sentence, which is shown as it comes.
 *
 * `id` absent: a new report. Needs `dashboard:write`; without it the page says
 * so (`BI-18D-AC8`), and the server refuses the save anyway.
 */
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
  return id ? <ExistingReportEditor id={id} /> : <ReportEditor view={null} />
}

function ExistingReportEditor({ id }: { id: string }) {
  const state = useService((s) => dashboardReportService.getReport(id, s), [id])
  if (state.status === "loading") return <LoadingSkeleton />
  if (state.status === "error") {
    if (state.error.code === "not_found") {
      return <EmptyState title="This report is not available" description="It may have been deleted, or it is in a folder you cannot open." />
    }
    return <ErrorState error={state.error} onRetry={state.reload} />
  }
  return <ReportEditor view={state.data} />
}

function ReportEditor({ view }: { view: ReportView | null }) {
  const router = useRouter()
  const { resolvedTheme } = useTheme()
  const auth = useAuth()
  const me = auth.user?.id
  const foldersState = useService((s) => dashboardService.listFolders(s), [])
  const folders = React.useMemo(
    () => folderChoices(labelledFolders(foldersState.data ?? [], me), me),
    [foldersState.data, me],
  )
  const [title, setTitle] = React.useState(view?.report.title ?? "")
  const [folderId, setFolderId] = React.useState(view?.report.folderId || NO_FOLDER)
  const [blocks, setBlocks] = React.useState<ReportBlock[]>(view?.report.blocks ?? [])
  const [picked, setPicked] = React.useState<Picked>({})
  const [pickerAt, setPickerAt] = React.useState<number | null>(null)
  const [refusal, setRefusal] = React.useState<string | null>(null)

  const save = useServiceAction(
    withNotify({ success: "Report saved", error: "Could not save the report" }, (signal) => {
      const input = {
        title: title.trim(),
        folderId: folderId === NO_FOLDER ? "" : folderId,
        blocks: blocksForSave(blocks),
      }
      return view
        ? dashboardReportService.updateReport(view.report.id, input, signal)
        : dashboardReportService.createReport(input, signal)
    }),
  )

  const charts = chartCount(blocks)
  const badLink = blocks.some((b) => b.type === "link" && !safeReportUrl(b.url))
  const canSave = title.trim().length > 0 && charts <= MAX_REPORT_CHARTS && !badLink && save.status !== "pending"
  const cancelHref = view ? `/dashboards/reports/${encodeURIComponent(view.report.id)}` : REPORTS_TAB_HREF

  function add(after: number, type: ReportBlock["type"]) {
    if (type === "chart") {
      setPickerAt(after)
      return
    }
    setBlocks((prev) => insertBlock(prev, after, emptyBlock(type)))
  }

  function onPick(chart: PickedChart) {
    const at = pickerAt ?? blocks.length - 1
    setPicked((prev) => ({ ...prev, [chart.chart]: { title: chart.title, boardName: chart.boardName } }))
    setBlocks((prev) => insertBlock(prev, at, { type: "chart", chart: chart.chart, board: chart.board }))
    setPickerAt(null)
  }

  async function submit() {
    setRefusal(null)
    const saved = await save.run()
    if (saved) {
      router.push(`/dashboards/reports/${encodeURIComponent(saved.id)}`)
    }
  }

  React.useEffect(() => {
    if (save.status === "error") setRefusal(save.error.message)
  }, [save.status, save.error])

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-5">
      <Link
        href={cancelHref}
        className="inline-flex w-fit items-center gap-1 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <ArrowLeft className="size-3" aria-hidden />
        {view ? "Back to the report" : "Reports"}
      </Link>
      <PageHeader
        title={view ? "Edit report" : "New report"}
        description="Text and live charts from your dashboards, in the order you place them."
        actions={
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" render={<Link href={cancelHref} />}>
              Cancel
            </Button>
            <Button size="sm" disabled={!canSave} onClick={() => void submit()}>
              {save.status === "pending" ? "Saving…" : "Save"}
            </Button>
          </div>
        }
      />

      <div className="grid gap-3 sm:grid-cols-[1fr_16rem]">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="report-title">Title</Label>
          <Input
            id="report-title"
            autoFocus={!view}
            value={title}
            maxLength={200}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Visitors this quarter"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label>Folder</Label>
          <Select value={folderId} onValueChange={(v) => setFolderId(String(v ?? NO_FOLDER))}>
            <SelectTrigger className="w-full" aria-label="Folder">
              <SelectValue>{folderOptionText(folders, folderId === NO_FOLDER ? "" : folderId, "No folder")}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <FolderOptions folders={folders} noneValue={NO_FOLDER} noneLabel="No folder" />
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>
          {blocks.length} {blocks.length === 1 ? "block" : "blocks"}
        </span>
        <span className={cn("tabular-nums", charts >= MAX_REPORT_CHARTS && "font-medium text-foreground")} data-testid="chart-count">
          {charts} of {MAX_REPORT_CHARTS} charts
        </span>
      </div>

      {refusal ? (
        <p role="alert" className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          {refusal}
        </p>
      ) : null}

      <ReportingProvider reporting={view?.reporting ?? undefined}>
        <ol className="flex flex-col gap-3" aria-label="Blocks">
          {blocks.length === 0 ? (
            <li className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
              Start with a heading or a paragraph, then add charts from your dashboards.
            </li>
          ) : null}
          {blocks.map((block, i) => (
            <li key={i} className="group flex flex-col gap-2 rounded-xl border bg-card p-3">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-medium text-muted-foreground">{blockLabel(block)}</span>
                <div className="flex items-center gap-0.5">
                  <Button variant="ghost" size="icon-sm" aria-label="Move up" disabled={i === 0} onClick={() => setBlocks((b) => moveBlock(b, i, -1))}>
                    <ArrowUp />
                  </Button>
                  <Button variant="ghost" size="icon-sm" aria-label="Move down" disabled={i === blocks.length - 1} onClick={() => setBlocks((b) => moveBlock(b, i, 1))}>
                    <ArrowDown />
                  </Button>
                  <Button variant="ghost" size="icon-sm" aria-label="Remove block" onClick={() => setBlocks((b) => removeBlock(b, i))}>
                    <Trash2 />
                  </Button>
                </div>
              </div>
              <BlockFields
                block={block}
                index={i}
                onChange={(next) => setBlocks((b) => replaceBlock(b, i, next))}
                entry={block.type === "chart" ? view?.charts[block.chart] : undefined}
                picked={block.type === "chart" ? picked[block.chart] : undefined}
                dark={resolvedTheme === "dark"}
              />
              <AddBlockMenu label="Add below" chartsFull={!canAddChart(blocks)} onAdd={(type) => add(i, type)} subtle />
            </li>
          ))}
        </ol>
      </ReportingProvider>

      <AddBlockMenu label="Add block" chartsFull={!canAddChart(blocks)} onAdd={(type) => add(blocks.length - 1, type)} />

      <ReportChartPicker open={pickerAt !== null} onOpenChange={(open) => !open && setPickerAt(null)} onPick={onPick} />
    </div>
  )
}

function blockLabel(block: ReportBlock): string {
  switch (block.type) {
    case "heading":
      return "Heading"
    case "paragraph":
      return "Paragraph"
    case "list":
      return block.ordered ? "Numbered list" : "Bulleted list"
    case "link":
      return "Link"
    case "chart":
      return "Chart"
  }
}

function AddBlockMenu({
  label,
  chartsFull,
  onAdd,
  subtle = false,
}: {
  readonly label: string
  readonly chartsFull: boolean
  readonly onAdd: (type: ReportBlock["type"]) => void
  readonly subtle?: boolean
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant={subtle ? "ghost" : "outline"}
            size="sm"
            className={cn(subtle && "self-start text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100 data-[popup-open]:opacity-100")}
          />
        }
      >
        <PlusIcon data-icon="inline-start" />
        {label}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-52">
        <DropdownMenuItem onClick={() => onAdd("heading")}>
          <Heading className="size-4" /> Heading
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onAdd("paragraph")}>
          <Pilcrow className="size-4" /> Paragraph
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onAdd("list")}>
          <List className="size-4" /> List
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => onAdd("link")}>
          <Link2 className="size-4" /> Link
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={chartsFull} onClick={() => onAdd("chart")}>
          <BarChart3 className="size-4" />
          {chartsFull ? TOO_MANY_CHARTS : "Chart from a dashboard…"}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function BlockFields({
  block,
  index,
  onChange,
  entry,
  picked,
  dark,
}: {
  readonly block: ReportBlock
  readonly index: number
  readonly onChange: (block: ReportBlock) => void
  readonly entry: ReportChartEntry | undefined
  readonly picked: { title: string; boardName: string } | undefined
  readonly dark: boolean
}) {
  const fieldId = `report-block-${index}`
  switch (block.type) {
    case "heading":
      return (
        <div className="flex gap-2">
          <Input
            aria-label="Heading text"
            value={block.text}
            onChange={(e) => onChange({ ...block, text: e.target.value })}
            placeholder="Heading"
            className="font-semibold"
          />
          <Select value={String(block.level)} onValueChange={(v) => onChange({ ...block, level: Number(v) as 1 | 2 | 3 })}>
            <SelectTrigger className="w-28" aria-label="Heading size">
              <SelectValue>{block.level === 1 ? "Large" : block.level === 3 ? "Small" : "Medium"}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="1">Large</SelectItem>
              <SelectItem value="2">Medium</SelectItem>
              <SelectItem value="3">Small</SelectItem>
            </SelectContent>
          </Select>
        </div>
      )
    case "paragraph":
      return (
        <Textarea
          aria-label="Paragraph text"
          value={block.text}
          onChange={(e) => onChange({ ...block, text: e.target.value })}
          placeholder="What the numbers say"
          rows={3}
        />
      )
    case "list":
      return (
        <div className="flex flex-col gap-2">
          <Textarea
            aria-label="List items, one per line"
            value={listText(block.items)}
            onChange={(e) => onChange({ ...block, items: listItems(e.target.value) })}
            placeholder="One item per line"
            rows={3}
          />
          <label htmlFor={`${fieldId}-ordered`} className="flex w-fit items-center gap-2 text-xs text-muted-foreground">
            <Switch id={`${fieldId}-ordered`} checked={block.ordered === true} onCheckedChange={(on) => onChange({ ...block, ordered: on })} />
            Numbered
          </label>
        </div>
      )
    case "link": {
      const bad = !safeReportUrl(block.url)
      return (
        <div className="grid gap-2 sm:grid-cols-2">
          <Input aria-label="Link text" value={block.text} onChange={(e) => onChange({ ...block, text: e.target.value })} placeholder="Words to show" />
          <div className="flex flex-col gap-1">
            <Input
              aria-label="Link address"
              aria-invalid={bad}
              value={block.url}
              onChange={(e) => onChange({ ...block, url: e.target.value })}
              placeholder="https://"
            />
            {bad ? <p className="text-xs text-destructive">Start the address with https://, http://, mailto: or /.</p> : null}
          </div>
        </div>
      )
    }
    case "chart":
      if (entry) return <ReportChart entry={entry} dark={dark} />
      return (
        <div className="flex items-center gap-2 rounded-lg bg-muted/40 px-3 py-4 text-sm">
          <BarChart3 className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <span className="min-w-0 truncate font-medium">{picked?.title ?? block.chart}</span>
          {picked?.boardName ? <span className="text-xs text-muted-foreground">from {picked.boardName}</span> : null}
          <span className="ml-auto text-xs text-muted-foreground">Drawn live once saved</span>
        </div>
      )
  }
}
