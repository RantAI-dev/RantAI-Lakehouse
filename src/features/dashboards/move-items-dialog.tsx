"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Select, SelectContent, SelectTrigger, SelectValue } from "@/components/ui/select"
import { folderOptionText } from "@/lib/dashboard-organise"
import type { FolderLike } from "@/lib/folder-tree"
import { readableErrorMessage } from "@/lib/notify-message"
import { dashboardReportService, dashboardService } from "@/services"
import type { MoveEffect } from "@/services"
import { MoveEffectNote } from "./folder-dialogs"
import { FolderOptions } from "./folder-options"
import type { LibraryItem } from "./library-items"

const ROOT = "__root__"

/** The outcome of a move of several items: how many moved, and each refusal with the server's sentence. */
export type MoveOutcome = { moved: number; refused: { name: string; reason: string }[] }

/**
 * Moves dashboards and reports into one folder, through the services that
 * already move one (`moveBoard`, and `updateReport` with a folder), one after
 * the other; no new API (`BI-59` T5). A refusal stops nothing: the rest still
 * move and the dialog names what was refused and why.
 */
export async function moveItems(items: readonly LibraryItem[], folderId: string, signal?: AbortSignal): Promise<MoveOutcome> {
  const out: MoveOutcome = { moved: 0, refused: [] }
  for (const item of items) {
    if (item.folderId === folderId) continue
    try {
      if (item.kind === "dashboard") await dashboardService.moveBoard(item.id, folderId, signal)
      else if (item.kind === "report") await dashboardReportService.updateReport(item.id, { folderId }, signal)
      else continue
      out.moved += 1
    } catch (err) {
      out.refused.push({ name: item.name, reason: readableErrorMessage(err instanceof Error ? err.message : String(err)) })
    }
  }
  return out
}

/**
 * "Move to folder" for one report, or for the rows ticked on the list. A
 * dashboard's move changes who sees it, so each dashboard whose access would
 * change is named with who gains and who loses (feature B7, as the
 * one-dashboard dialog does). A personal space holds dashboards only, so it is
 * not offered while a report is among the items.
 */
export function MoveItemsDialog({
  items, folders, onOpenChange, onDone,
}: {
  readonly items: readonly LibraryItem[]
  /** The folders the caller may file into. */
  readonly folders: FolderLike[]
  readonly onOpenChange: (open: boolean) => void
  readonly onDone: (outcome: MoveOutcome) => void
}) {
  const hasReport = items.some((i) => i.kind === "report")
  const choices = React.useMemo(() => (hasReport ? folders.filter((f) => !f.ownerId) : folders), [folders, hasReport])
  const single = items.length === 1 ? items[0] : null
  const [target, setTarget] = React.useState(single?.folderId || ROOT)
  const [busy, setBusy] = React.useState(false)
  const [refused, setRefused] = React.useState<MoveOutcome["refused"]>([])
  const destination = target === ROOT ? "" : target

  const [effects, setEffects] = React.useState<{ name: string; effect: MoveEffect }[]>([])
  React.useEffect(() => {
    setEffects([])
    const boards = items.filter((i) => i.kind === "dashboard" && i.folderId !== destination)
    if (boards.length === 0) return
    const ctl = new AbortController()
    void Promise.all(
      boards.map((b) =>
        dashboardService
          .previewMove(b.id, destination, ctl.signal)
          .then((effect) => ({ name: b.name, effect }))
          .catch(() => null),
      ),
    ).then((all) => {
      if (ctl.signal.aborted) return
      setEffects(all.filter((e): e is { name: string; effect: MoveEffect } => e !== null && changes(e.effect)))
    })
    return () => ctl.abort()
  }, [items, destination])

  const nothingToDo = items.every((i) => i.folderId === destination)

  async function move() {
    setBusy(true)
    setRefused([])
    const outcome = await moveItems(items, destination)
    setBusy(false)
    if (outcome.refused.length > 0) setRefused(outcome.refused)
    onDone(outcome)
    if (outcome.refused.length === 0) onOpenChange(false)
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>Move to folder</DialogTitle>
          <DialogDescription>
            {single ? single.name : `${items.length.toLocaleString("en-US")} items`}
          </DialogDescription>
        </DialogHeader>
        {/* DESIGN.md §3 (owner, 2026-11-14): a visible label, and the control as wide as the dialog. */}
        <p id="move-items-folder" className="-mb-1 text-sm font-medium">Folder</p>
        <Select value={target} onValueChange={(v) => setTarget(v ?? ROOT)}>
          <SelectTrigger aria-label="Folder" aria-describedby="move-items-folder" className="w-full">
            <SelectValue>{folderOptionText(choices, destination, "Top level (no folder)")}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <FolderOptions folders={choices} noneValue={ROOT} noneLabel="Top level (no folder)" />
          </SelectContent>
        </Select>
        {hasReport && choices.length < folders.length ? (
          <p className="text-xs text-muted-foreground">A personal space holds dashboards only, so it is not offered for reports.</p>
        ) : null}
        {effects.map(({ name, effect }) => (
          <div key={name} className="grid gap-1">
            {items.length > 1 ? <p className="text-xs font-medium">{name}</p> : null}
            <MoveEffectNote effect={effect} />
          </div>
        ))}
        {refused.length > 0 ? (
          <ul role="alert" className="grid gap-1 text-sm text-destructive">
            {refused.map((r) => <li key={r.name}>{r.name}: {r.reason}</li>)}
          </ul>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" onClick={() => void move()} disabled={busy || nothingToDo}>
            {busy ? "Moving…" : "Move"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function changes(effect: MoveEffect): boolean {
  return effect.gains.length > 0 || effect.loses.length > 0 || effect.startingPointGains || effect.startingPointLoses
}
