"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { applySuggestion, callAtCaret, suggest } from "@/lib/formula-assist"
import { draftFromForm, emptyMetricForm, formFromMetric, formProblems, usageSentence, MAX_DESCRIPTION } from "@/lib/metric-form"
import { cn } from "@/lib/utils"
import { calcFieldService, metricService } from "@/services"
import type { FormulaFunction } from "@/services/contracts/calc-fields"
import type { Metric, MetricCheck, MetricSource, Segment } from "@/services/contracts/metrics"
import type { FilterDef } from "@/services/clients/bi-store"
import { ChartFilters } from "./chart-shape-section"
import { useSourceChoices, useSourceColumns } from "./use-source-columns"

const CHECK_DELAY_MS = 600
const NO_VALUE = "__none__"
const GRAINS = ["day", "week", "month", "quarter", "year"] as const

/**
 * Define or change a metric (BI-2): a name and source (kept for good once saved), a label and description,
 * an aggregate formula, filters and segments that narrow this metric's number only, and the date column and
 * default period its comparisons use. The server is the only judge: the check runs once typing pauses and
 * reads the number as the person editing (their access applies). Changing a metric that charts use asks
 * first, naming them.
 */
export function MetricDialog({ open, onOpenChange, existing, onChanged }: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly existing?: Metric
  readonly onChanged: () => void
}) {
  const choices = useSourceChoices(open && !existing)
  const [sourceValue, setSourceValue] = React.useState("")
  const [name, setName] = React.useState("")
  const [form, setForm] = React.useState(emptyMetricForm())
  const [caret, setCaret] = React.useState(0)
  const [check, setCheck] = React.useState<MetricCheck | "pending" | null>(null)
  const [functions, setFunctions] = React.useState<FormulaFunction[]>([])
  const [segments, setSegments] = React.useState<Segment[]>([])
  const [problem, setProblem] = React.useState<string | null>(null)
  const [confirm, setConfirm] = React.useState<string | null>(null)
  const [busy, setBusy] = React.useState(false)
  const area = React.useRef<HTMLTextAreaElement>(null)

  const source = React.useMemo<MetricSource | null>(() => {
    if (existing) return existing.sourceKind === "mart" ? { mart: existing.sourceId } : { source: existing.sourceId }
    return choices.find((c) => c.value === sourceValue)?.source ?? null
  }, [existing, choices, sourceValue])
  const { columns, calculated } = useSourceColumns(source)

  React.useEffect(() => {
    if (!open) return
    setSourceValue("")
    setName(existing?.name ?? "")
    setForm(existing ? formFromMetric(existing) : emptyMetricForm())
    setCaret((existing?.formula ?? "").length)
    setCheck(null); setProblem(null); setConfirm(null)
    void calcFieldService.functions().then(setFunctions).catch(() => setFunctions([]))
    void metricService.listSegments().then(setSegments).catch(() => setSegments([]))
  }, [open, existing])

  const sourceKey = source ? (source.mart ?? `sql:${source.source}`) : ""
  const mine = segments.filter((s) => (source?.mart !== undefined ? s.sourceKind === "mart" && s.sourceId === source.mart : s.sourceKind === "sql_source" && s.sourceId === source?.source))
  const problems = formProblems(form, !existing, name)
  const body = JSON.stringify(draftFromForm(form))

  // Check once typing pauses; a newer edit cancels the answer to an older one.
  React.useEffect(() => {
    if (!open || !source || !form.formula.trim() || !form.label.trim()) { setCheck(null); return }
    setCheck("pending")
    const ctl = new AbortController()
    const timer = setTimeout(() => {
      metricService.validate(source, draftFromForm(form), { name: name || undefined, value: true, signal: ctl.signal })
        .then(setCheck)
        .catch((e: unknown) => {
          if (ctl.signal.aborted) return
          setCheck(null)
          setProblem(e instanceof Error ? e.message : "The metric could not be checked.")
        })
    }, CHECK_DELAY_MS)
    return () => { clearTimeout(timer); ctl.abort() }
    // The form is read through `body`, its serialised form; `source` through `sourceKey`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, body, sourceKey, name])

  const names = React.useMemo(() => ({
    columns: columns.map((c) => c.name),
    fields: calculated.map((f) => ({ name: f.name, formula: f.formula })),
    params: [],
  }), [columns, calculated])
  const hints = suggest(form.formula, caret, names, functions)
  const call = callAtCaret(form.formula, caret, functions)
  const valid = check !== null && check !== "pending" && check.ok && problems.length === 0
  const dates = columns.filter((c) => c.kind === "date" || c.kind === "datetime")
  const set = (patch: Partial<typeof form>) => { setForm((f) => ({ ...f, ...patch })); setConfirm(null) }

  function pick(i: number) {
    const out = applySuggestion(form.formula, hints, hints.items[i])
    set({ formula: out.text })
    setCaret(out.caret)
    requestAnimationFrame(() => { area.current?.focus(); area.current?.setSelectionRange(out.caret, out.caret) })
  }

  async function save(confirmed: boolean) {
    setBusy(true); setProblem(null)
    try {
      if (existing) {
        // Changing a metric reaches every chart that uses it: say which, and ask first.
        if (!confirmed && (existing.usedBy ?? 0) > 0) {
          const sentence = usageSentence(await metricService.usage(existing.id))
          if (sentence) { setConfirm(sentence); return }
        }
        await metricService.update(existing.id, draftFromForm(form))
      } else if (source) {
        await metricService.create(source, name, draftFromForm(form))
      }
      onChanged()
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The metric could not be saved.")
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    if (!existing) return
    setBusy(true); setProblem(null)
    try {
      await metricService.remove(existing.id)
      onChanged()
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The metric could not be deleted.")
    } finally {
      setBusy(false)
    }
  }

  const value = check && check !== "pending" && check.ok ? check.value : undefined
  const shown = value?.rows?.[0] ? Object.values(value.rows[0])[0] : undefined
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader><DialogTitle>{existing ? `Edit ${existing.label || existing.name}` : "New metric"}</DialogTitle></DialogHeader>
        <div className="grid gap-3">
          {existing ? null : (
            <div className="grid gap-1.5">
              <Label>Source</Label>
              <Select value={sourceValue} onValueChange={(v) => setSourceValue(v ?? "")}>
                <SelectTrigger className="w-full" aria-label="Source"><SelectValue placeholder="pick a table" /></SelectTrigger>
                <SelectContent>{choices.map((c) => <SelectItem key={c.value} value={c.value}>{c.label}</SelectItem>)}</SelectContent>
              </Select>
            </div>
          )}
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="metric-name">Name</Label>
              <Input id="metric-name" value={name} disabled={!!existing} placeholder="total_visitors" maxLength={64} onChange={(e) => setName(e.target.value.trim())} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="metric-label">Label</Label>
              <Input id="metric-label" value={form.label} placeholder="Total visitors" maxLength={100} onChange={(e) => set({ label: e.target.value })} />
            </div>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="metric-description">Description</Label>
            <Textarea id="metric-description" rows={2} value={form.description} maxLength={MAX_DESCRIPTION} onChange={(e) => set({ description: e.target.value })} />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="metric-formula">Formula</Label>
            <Textarea
              id="metric-formula" ref={area} value={form.formula} rows={3} spellCheck={false} className="font-mono text-sm" placeholder="Sum([visitors])"
              onChange={(e) => { set({ formula: e.target.value }); setCaret(e.target.selectionStart) }}
              onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
              onKeyUp={(e) => setCaret(e.currentTarget.selectionStart)}
            />
            {call ? <p className="text-xs text-muted-foreground"><span className="font-mono">{call.fn.signature}</span> {call.fn.help}</p> : null}
            {hints.items.length ? (
              <ul className="flex flex-wrap gap-1" aria-label="Suggestions">
                {hints.items.map((s, i) => (
                  <li key={`${s.kind}-${s.label}`}>
                    <button type="button" title={s.detail} className={cn("rounded border px-1.5 py-0.5 font-mono text-xs hover:bg-muted", s.kind === "function" && "text-primary")} onMouseDown={(e) => e.preventDefault()} onClick={() => pick(i)}>{s.label}</button>
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
          <ChartFilters
            value={form.filters as unknown as FilterDef[]}
            onChange={(next) => set({ filters: next as unknown as typeof form.filters })}
            columns={columns}
            disabled={!source}
          />
          {mine.length ? (
            <div className="grid gap-1.5">
              <Label>Segments</Label>
              <div className="flex flex-wrap gap-1.5">
                {mine.map((s) => {
                  const on = form.segments.includes(s.name)
                  return (
                    <Button key={s.id} type="button" size="sm" variant={on ? "secondary" : "outline"} aria-pressed={on}
                      onClick={() => set({ segments: on ? form.segments.filter((n) => n !== s.name) : [...form.segments, s.name] })}>
                      {s.label || s.name}
                    </Button>
                  )
                })}
              </div>
            </div>
          ) : null}
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Date column</Label>
              <Select value={form.timeColumn || NO_VALUE} items={{ [NO_VALUE]: "None" }} onValueChange={(v) => set({ timeColumn: !v || v === NO_VALUE ? "" : v, grain: !v || v === NO_VALUE ? "" : form.grain })}>
                <SelectTrigger className="w-full" aria-label="Date column"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_VALUE}>None</SelectItem>
                  {dates.map((c) => <SelectItem key={c.name} value={c.name}>{c.name}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Default period</Label>
              <Select value={form.grain || NO_VALUE} items={{ [NO_VALUE]: "None" }} disabled={!form.timeColumn} onValueChange={(v) => set({ grain: !v || v === NO_VALUE ? "" : v })}>
                <SelectTrigger className="w-full" aria-label="Default period"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_VALUE}>None</SelectItem>
                  {GRAINS.map((g) => <SelectItem key={g} value={g}>{g}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
          </div>
          {check === "pending" ? <p className="text-xs text-muted-foreground">Checking…</p> : null}
          {check && check !== "pending" && !check.ok ? <p role="alert" className="text-sm text-destructive">{check.error.message}</p> : null}
          {value && !value.error && shown !== undefined ? (
            <p className="text-sm" aria-label="Value now">Value now: <span className="font-semibold tabular-nums">{String(shown)}</span></p>
          ) : null}
          {value?.error ? <p role="alert" className="text-sm text-destructive">{value.error}{value.errorId ? ` (${value.errorId})` : ""}</p> : null}
          {confirm ? (
            <div role="alert" className="grid gap-2 rounded-md border border-border bg-muted/40 p-2.5 text-sm">
              <p>{confirm}</p>
              <div className="flex gap-2">
                <Button type="button" size="sm" disabled={busy} onClick={() => void save(true)}>Change them all</Button>
                <Button type="button" size="sm" variant="outline" onClick={() => setConfirm(null)}>Keep editing</Button>
              </div>
            </div>
          ) : null}
          {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
        </div>
        <DialogFooter>
          {existing ? <Button type="button" variant="outline" className="sm:mr-auto" disabled={busy} onClick={() => void remove()}>Delete</Button> : null}
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button type="button" disabled={!valid || busy || confirm !== null} onClick={() => void save(false)}>{existing ? "Save" : "Create"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
