"use client"

import * as React from "react"
import { Plus } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useService } from "@/hooks/use-service"
import { metricService } from "@/services"
import type { FilterDef } from "@/services/clients/bi-store"
import type { Segment } from "@/services/contracts/metrics"
import { ChartFilters } from "./chart-shape-section"
import { SourceChoiceLabel, sourceChoiceItems } from "./source-choice-label"
import { useSourceChoices, useSourceColumns } from "./use-source-columns"

/**
 * Segments (BI-2): a named list of filters on one table, such as "domestic visitors", used in
 * metrics and as a filter of any chart on that table. Its name and table stay once saved; deleting
 * one that a chart or metric uses is refused with the server's sentence.
 */
export function SegmentsDialog({ open, onOpenChange }: { readonly open: boolean; readonly onOpenChange: (open: boolean) => void }) {
  const state = useService((signal) => metricService.listSegments(signal), [open])
  const choices = useSourceChoices(open)
  const [editing, setEditing] = React.useState<Segment | "new" | null>(null)
  const [sourceValue, setSourceValue] = React.useState("")
  const [name, setName] = React.useState("")
  const [label, setLabel] = React.useState("")
  const [filters, setFilters] = React.useState<FilterDef[]>([])
  const [busy, setBusy] = React.useState(false)
  const [problem, setProblem] = React.useState<string | null>(null)

  const existing = editing && editing !== "new" ? editing : undefined
  const source = existing
    ? (existing.sourceKind === "mart" ? { mart: existing.sourceId } : { source: existing.sourceId })
    : (choices.find((c) => c.value === sourceValue)?.source ?? null)
  const { columns } = useSourceColumns(source)

  React.useEffect(() => { if (!open) { setEditing(null); setProblem(null) } }, [open])

  const edit = (target: Segment | "new") => {
    setEditing(target); setProblem(null); setSourceValue("")
    setName(target === "new" ? "" : target.name)
    setLabel(target === "new" ? "" : target.label)
    setFilters(target === "new" ? [] : (target.filters as unknown as FilterDef[]))
  }
  const ready = label.trim() !== "" && filters.length > 0 && (existing || (source !== null && /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name)))

  async function save() {
    setBusy(true); setProblem(null)
    try {
      const draft = { label: label.trim(), filters: filters as unknown as Segment["filters"] }
      if (existing) await metricService.updateSegment(existing.id, draft)
      else if (source) await metricService.createSegment(source, name, draft)
      setEditing(null)
      state.reload()
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The segment could not be saved.")
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    if (!existing) return
    setBusy(true); setProblem(null)
    try {
      await metricService.removeSegment(existing.id)
      setEditing(null)
      state.reload()
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The segment could not be deleted.")
    } finally {
      setBusy(false)
    }
  }

  const segments = state.data ?? []
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader><DialogTitle>{editing === null ? "Segments" : existing ? `Edit ${existing.label || existing.name}` : "New segment"}</DialogTitle></DialogHeader>
        {editing === null ? (
          <div className="grid gap-2">
            {segments.length === 0 && state.status !== "loading" ? <p className="text-sm text-muted-foreground">No segments yet.</p> : null}
            <ul className="grid gap-1">
              {segments.map((s) => (
                <li key={s.id} className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-sm">
                  <span className="min-w-0 flex-1 truncate"><span className="font-medium">{s.label || s.name}</span> <span className="font-mono text-xs text-muted-foreground">{s.name} · {s.sourceId}</span></span>
                  <Button type="button" size="xs" variant="ghost" onClick={() => edit(s)}>Edit</Button>
                </li>
              ))}
            </ul>
            {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Close</Button>
              <Button type="button" onClick={() => edit("new")}><Plus className="size-4" aria-hidden /> New segment</Button>
            </DialogFooter>
          </div>
        ) : (
          <div className="grid gap-3">
            {existing ? null : (
              <div className="grid gap-1.5">
                <Label>Source</Label>
                <Select value={sourceValue} items={sourceChoiceItems(choices)} onValueChange={(v) => { setSourceValue(v ?? ""); setFilters([]) }}>
                  <SelectTrigger className="w-full" aria-label="Source"><SelectValue placeholder="pick a table" /></SelectTrigger>
                  <SelectContent>{choices.map((c) => <SelectItem key={c.value} value={c.value}><SourceChoiceLabel choice={c} /></SelectItem>)}</SelectContent>
                </Select>
              </div>
            )}
            <div className="grid grid-cols-2 gap-3">
              <div className="grid gap-1.5">
                <Label htmlFor="segment-name">Name</Label>
                <Input id="segment-name" value={name} disabled={!!existing} placeholder="domestic" maxLength={64} onChange={(e) => setName(e.target.value.trim())} />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="segment-label">Label</Label>
                <Input id="segment-label" value={label} placeholder="Domestic visitors" maxLength={100} onChange={(e) => setLabel(e.target.value)} />
              </div>
            </div>
            <ChartFilters value={filters} onChange={setFilters} columns={columns} disabled={!source} />
            {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
            <DialogFooter>
              {existing ? <Button type="button" variant="outline" className="sm:mr-auto" disabled={busy} onClick={() => void remove()}>Delete</Button> : null}
              <Button type="button" variant="outline" onClick={() => setEditing(null)}>Back</Button>
              <Button type="button" disabled={!ready || busy} onClick={() => void save()}>{existing ? "Save" : "Create"}</Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
