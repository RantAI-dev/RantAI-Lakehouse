"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useServiceAction } from "@/hooks/use-service"
import { folderOptionText, isPersonalFolder } from "@/lib/dashboard-organise"
import type { FolderLike } from "@/lib/folder-tree"
import { withNotify } from "@/lib/notify"
import { dashboardTemplateService } from "@/services"
import type { DashboardTemplate, StartedBoard } from "@/services"
import { FolderOptions } from "./folder-options"
import { PRIVATE_HINT } from "./private-badge"

/** Select value for "no folder" (base-ui needs a non-empty value). */
const NO_FOLDER = "__none__"

/**
 * "Save as template" from a dashboard's ⋯ menu (`BI-18` part D, T6, decisions
 * T1 and T4): a name, then a copy of the dashboard's definition (layout,
 * charts, filters, text tiles; no data) is saved for everyone with
 * `dashboard:write` to start from. The dashboard itself is not changed.
 */
export function SaveTemplateDialog({
  open,
  onOpenChange,
  board,
  boardName,
  onSaved,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly board: string
  readonly boardName: string
  readonly onSaved?: (template: DashboardTemplate) => void
}) {
  const [name, setName] = React.useState(boardName)
  React.useEffect(() => {
    if (open) setName(boardName)
  }, [open, boardName])
  const save = useServiceAction(
    withNotify({ success: "Template saved", error: "Could not save the template" }, (signal, value: string) =>
      dashboardTemplateService.saveTemplate({ board, name: value }, signal),
    ),
  )
  const clean = name.trim()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Save as template</DialogTitle>
          <DialogDescription>
            Saves this dashboard&apos;s layout, charts, filters and text tiles, without any data. Anyone who can edit
            dashboards can start a new one from it on the Templates tab.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="template-name">Template name</Label>
          <Input id="template-name" autoFocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} />
        </div>
        {save.status === "error" ? (
          <p role="alert" className="text-sm text-destructive">{save.error.message}</p>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={!clean || save.status === "pending"}
            onClick={() => {
              void (async () => {
                const saved = await save.run(clean)
                if (saved) {
                  onSaved?.(saved)
                  onOpenChange(false)
                }
              })()
            }}
          >
            {save.status === "pending" ? "Saving…" : "Save template"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * "Use template" (`BI-18` part D, T6, decisions T2 and T4): a name and a
 * folder, then a new, independent dashboard made from the template opens.
 * Charts the server could not copy (a SQL source the person may not use) are
 * listed with its reason; a chart whose table is gone is kept and shows the
 * dashboard's own "not available" sentence on its tile.
 */
export function UseTemplateDialog({
  template,
  folders,
  onOpenChange,
  onStarted,
}: {
  readonly template: DashboardTemplate | null
  readonly folders: FolderLike[]
  readonly onOpenChange: (open: boolean) => void
  readonly onStarted: (started: StartedBoard) => void
}) {
  const [name, setName] = React.useState("")
  const [folderId, setFolderId] = React.useState(NO_FOLDER)
  React.useEffect(() => {
    if (template) {
      setName(template.name)
      setFolderId(NO_FOLDER)
    }
  }, [template])
  const start = useServiceAction(
    withNotify(
      { success: "Dashboard created", error: "Could not make the dashboard" },
      (signal, id: string, input: { name: string; folderId: string }) =>
        dashboardTemplateService.useTemplate(id, input, signal),
    ),
  )
  const clean = name.trim()
  return (
    <Dialog open={template !== null} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>New dashboard from {template?.name ?? "template"}</DialogTitle>
          <DialogDescription>
            The new dashboard is a copy: changing it does not change the template.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="template-board-name">Name</Label>
            <Input id="template-board-name" autoFocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} />
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
            {isPersonalFolder(folders, folderId) ? <p className="text-xs text-muted-foreground">{PRIVATE_HINT}</p> : null}
          </div>
        </div>
        {start.status === "error" ? (
          <p role="alert" className="text-sm text-destructive">{start.error.message}</p>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={!clean || !template || start.status === "pending"}
            onClick={() => {
              if (!template) return
              void (async () => {
                const started = await start.run(template.id, {
                  name: clean,
                  folderId: folderId === NO_FOLDER ? "" : folderId,
                })
                if (started) onStarted(started)
              })()
            }}
          >
            {start.status === "pending" ? "Making it…" : "Create dashboard"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
