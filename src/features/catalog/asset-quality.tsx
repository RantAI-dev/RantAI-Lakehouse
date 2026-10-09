"use client"

import * as React from "react"
import Link from "next/link"
import { Pencil, Play, Plus, Trash2 } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { CheckBadge, Pill } from "@/components/patterns/status-badge"
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAuth } from "@/features/auth/auth-provider"
import { EditRuleDialog } from "@/features/governance/quality-rule-edit-dialog"
import { useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { notifyError, notifySuccess } from "@/lib/notify"
import { SEVERITY_LABEL, type Severity } from "@/lib/status"
import { governanceService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import type { CreateQualityRuleInput } from "@/services/contracts/governance"
import { lineageKey } from "./asset-lineage"
import { qualitySummary } from "./asset-overview"

type Check = AssetDetail["qualityChecks"][number]

const SEVERITY_TONE = { critical: "destructive", high: "destructive", medium: "warning" } as const

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

/** What the evaluator can check, and the quality dimension each one is. */
const KINDS = {
  rows: { label: "Row count is at least…", dimension: "volume", column: false },
  "not-null": { label: "Column is filled in", dimension: "completeness", column: true },
  unique: { label: "Column has no repeated values", dimension: "uniqueness", column: true },
  range: { label: "Column stays within a range", dimension: "validity", column: true },
} as const

type Kind = keyof typeof KINDS

type RuleForm = {
  kind: Kind
  column: string
  /** `rows`: the minimum row count. */
  rows: string
  /** `not-null`: the minimum share, 0–100. */
  percent: string
  /** `range`: either bound may be left empty. */
  min: string
  max: string
}

const isNumber = (v: string) => v.trim() !== "" && Number.isFinite(Number(v))

/**
 * The threshold the evaluator reads (`routes::quality::parse_threshold`)
 * for what the form says, or `null` while the form is incomplete. Built
 * here so a rule authored on this page is always one that can be run.
 */
export function ruleThreshold(f: RuleForm): string | null {
  const column = f.column.trim()
  switch (f.kind) {
    case "rows":
      return /^\d+$/.test(f.rows.trim()) ? `rows >= ${f.rows.trim()}` : null
    case "not-null": {
      if (!column || !isNumber(f.percent)) return null
      const p = Number(f.percent)
      if (p < 0 || p > 100) return null
      return p === 100 ? `${column} not null` : `${column} not null >= ${p}%`
    }
    case "unique":
      return column ? `${column} unique` : null
    case "range": {
      if (!column) return null
      const [min, max] = [f.min.trim(), f.max.trim()]
      if ((min && !isNumber(min)) || (max && !isNumber(max))) return null
      if (min && max) return `${column} between ${Number(min)} and ${Number(max)}`
      if (min) return `${column} >= ${Number(min)}`
      if (max) return `${column} <= ${Number(max)}`
      return null
    }
  }
}

function defaultName(table: string, f: RuleForm) {
  const what = { rows: "row_count", "not-null": "not_null", unique: "unique", range: "range" }[f.kind]
  return [table, KINDS[f.kind].column ? f.column : null, what].filter(Boolean).join("_")
}

function AddRuleDialog({
  asset: a,
  onClose,
  onCreated,
}: {
  asset: AssetDetail
  onClose: () => void
  onCreated: () => void
}) {
  // The evaluator addresses a column by a plain identifier.
  const columns = a.schema.map((c) => c.name).filter((n) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(n))
  const key = lineageKey(a)
  const [form, setForm] = React.useState<RuleForm>({
    kind: "not-null",
    column: columns[0] ?? "",
    rows: "1",
    percent: "100",
    min: "",
    max: "",
  })
  const [name, setName] = React.useState<string | null>(null)
  const [severity, setSeverity] = React.useState<Severity>("medium")
  const create = useServiceAction((signal, input: CreateQualityRuleInput) =>
    governanceService.createQualityRule(input, signal)
  )
  const threshold = ruleThreshold(form)
  const table = key.split(".").pop() ?? key
  const ruleName = (name ?? defaultName(table, form)).trim()
  const set = (patch: Partial<RuleForm>) => setForm((f) => ({ ...f, ...patch }))

  async function submit() {
    if (!threshold || !ruleName) return
    const created = await create.run({
      name: ruleName,
      asset: key,
      dimension: KINDS[form.kind].dimension,
      threshold,
      severity,
    })
    if (created === null) return
    notifySuccess("Quality rule added")
    onCreated()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Add quality rule</DialogTitle>
          <DialogDescription>
            A check on <span className="font-mono">{key}</span> that can be run from this page.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="rule-kind">Check</Label>
            <select
              id="rule-kind"
              className={selectClassName}
              value={form.kind}
              onChange={(e) => set({ kind: e.target.value as Kind })}
            >
              {(Object.keys(KINDS) as Kind[]).map((k) => (
                <option key={k} value={k}>
                  {KINDS[k].label}
                </option>
              ))}
            </select>
          </div>
          {KINDS[form.kind].column ? (
            <div className="grid gap-1.5">
              <Label htmlFor="rule-column">Column</Label>
              <select
                id="rule-column"
                className={selectClassName}
                value={form.column}
                onChange={(e) => set({ column: e.target.value })}
              >
                {columns.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            </div>
          ) : null}
          {form.kind === "unique" && key.startsWith("bronze.") ? (
            <p className="text-xs text-muted-foreground">
              Bronze keeps every load, so a source loaded twice holds each row twice. Where the
              table records its loads, a value counts as repeated only when it repeats within
              one load.
            </p>
          ) : null}
          {form.kind === "rows" ? (
            <div className="grid gap-1.5">
              <Label htmlFor="rule-rows">Minimum rows</Label>
              <Input
                id="rule-rows"
                inputMode="numeric"
                value={form.rows}
                onChange={(e) => set({ rows: e.target.value })}
              />
            </div>
          ) : null}
          {form.kind === "not-null" ? (
            <div className="grid gap-1.5">
              <Label htmlFor="rule-percent">Rows that must have a value (%)</Label>
              <Input
                id="rule-percent"
                inputMode="decimal"
                value={form.percent}
                onChange={(e) => set({ percent: e.target.value })}
              />
            </div>
          ) : null}
          {form.kind === "range" ? (
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label htmlFor="rule-min">Minimum</Label>
                <Input
                  id="rule-min"
                  inputMode="decimal"
                  value={form.min}
                  onChange={(e) => set({ min: e.target.value })}
                  placeholder="no lower bound"
                />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="rule-max">Maximum</Label>
                <Input
                  id="rule-max"
                  inputMode="decimal"
                  value={form.max}
                  onChange={(e) => set({ max: e.target.value })}
                  placeholder="no upper bound"
                />
              </div>
            </div>
          ) : null}
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label htmlFor="rule-name">Name</Label>
              <Input id="rule-name" value={ruleName} onChange={(e) => setName(e.target.value)} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="rule-severity">Severity if it fails</Label>
              <select
                id="rule-severity"
                className={selectClassName}
                value={severity}
                onChange={(e) => setSeverity(e.target.value as Severity)}
              >
                {(Object.keys(SEVERITY_LABEL) as Severity[]).map((s) => (
                  <option key={s} value={s}>
                    {SEVERITY_LABEL[s]}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            {threshold ? (
              <>
                Threshold: <span className="font-mono text-foreground">{threshold}</span>
              </>
            ) : (
              "Fill in the check to see its threshold."
            )}
          </p>
          {create.error ? <p className="text-sm text-destructive">{create.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={!threshold || !ruleName || create.status === "pending"}
          >
            {create.status === "pending" ? "Adding…" : "Add rule"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Severity({ severity }: { severity?: string }) {
  if (!severity) return <span className="text-muted-foreground">—</span>
  const tone = SEVERITY_TONE[severity as keyof typeof SEVERITY_TONE] ?? "neutral"
  return <Pill tone={tone}>{severity}</Pill>
}

function Result({ q }: { q: Check }) {
  if (q.status !== null) {
    return (
      <div className="flex flex-col items-start gap-0.5">
        <CheckBadge status={q.status} />
        {q.value ? <span className="text-xs text-muted-foreground">{q.value}</span> : null}
      </div>
    )
  }
  // An authored rule the evaluator cannot read is not "not run yet": it
  // never will be, until its threshold is rewritten.
  return q.evaluable === false ? (
    <Pill tone="warning" title={q.hint ?? undefined}>
      Can&apos;t be run
    </Pill>
  ) : (
    <Pill tone="neutral">Not run yet</Pill>
  )
}

function CheckRow({
  q,
  onEdit,
  onDelete,
}: {
  q: Check
  onEdit?: (q: Check) => void
  onDelete?: (q: Check) => void
}) {
  return (
    <TableRow>
      <TableCell className="py-1.5 align-top">
        <div className="text-sm font-medium">{q.name}</div>
        <div className="text-xs text-muted-foreground">
          {q.origin === "observed" ? "Recorded by the quality job" : "Authored rule"}
        </div>
      </TableCell>
      <TableCell className="py-1.5 align-top text-xs">{q.dimension}</TableCell>
      <TableCell className="py-1.5 align-top text-xs">
        <span className="font-mono">{q.threshold || "—"}</span>
        {q.evaluable === false && q.hint ? (
          <div className="mt-0.5 max-w-sm text-muted-foreground">{q.hint}</div>
        ) : null}
      </TableCell>
      <TableCell className="py-1.5 align-top text-xs">
        <Severity severity={q.severity} />
      </TableCell>
      <TableCell className="py-1.5 align-top">
        <Result q={q} />
      </TableCell>
      <TableCell className="py-1.5 align-top text-xs text-muted-foreground">
        {q.lastRun === null ? "Never" : formatRelativeTime(q.lastRun)}
      </TableCell>
      <TableCell className="py-1 text-right align-top whitespace-nowrap">
        {/* Only an authored rule is this page's to change or delete; a
            recorded verdict belongs to the job that wrote it. */}
        {onEdit && q.origin === "rule" ? (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={`Edit rule ${q.name}`}
            onClick={() => onEdit(q)}
          >
            <Pencil />
          </Button>
        ) : null}
        {onDelete && q.origin === "rule" ? (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={`Delete rule ${q.name}`}
            onClick={() => onDelete(q)}
          >
            <Trash2 />
          </Button>
        ) : null}
      </TableCell>
    </TableRow>
  )
}

/**
 * The Quality tab: every check that names this asset with its latest
 * result, a way to run the authored ones now, and a way to add one that
 * can be run.
 */
export function AssetQuality({
  asset: a,
  onChanged,
}: {
  asset: AssetDetail
  /** Reloads the asset after checks ran or a rule was added. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const [adding, setAdding] = React.useState(false)
  const [running, setRunning] = React.useState(false)
  const [deleting, setDeleting] = React.useState<Check | null>(null)
  const [editing, setEditing] = React.useState<Check | null>(null)
  const remove = useServiceAction((signal, id: string) =>
    governanceService.deleteQualityRule(id, signal)
  )
  const canWrite = hasPermission("governance:write")

  async function confirmDelete() {
    if (!deleting) return
    const ok = await remove.run(deleting.id)
    if (ok === null) {
      notifyError("The rule could not be deleted", remove.error)
      return
    }
    notifySuccess(`Deleted rule ${deleting.name}`)
    setDeleting(null)
    onChanged()
  }
  const summary = qualitySummary(a.qualityChecks)
  const runnable = a.qualityChecks.filter((q) => q.origin === "rule" && q.evaluable !== false)
  const parts = [
    summary.passed > 0 ? `${summary.passed} passed` : null,
    summary.warning > 0 ? `${summary.warning} warning` : null,
    summary.failed > 0 ? `${summary.failed} failed` : null,
    summary.unevaluated > 0 ? `${summary.unevaluated} without a result` : null,
  ].filter((p): p is string => p !== null)

  async function runAll() {
    setRunning(true)
    let failures = 0
    let lastError: unknown = null
    // One at a time: each is a full aggregate over the table.
    for (const q of runnable) {
      try {
        await governanceService.runQualityRule(q.id)
      } catch (err) {
        failures += 1
        lastError = err
      }
    }
    setRunning(false)
    if (failures > 0) {
      notifyError(`${failures} of ${runnable.length} checks could not be run`, lastError)
    } else {
      notifySuccess(`Ran ${runnable.length} check${runnable.length === 1 ? "" : "s"}`)
    }
    onChanged()
  }

  return (
    <SectionCard
      size="sm"
      title="Quality checks"
      description={parts.length > 0 ? parts.join(" · ") : "Rules and recorded results for this asset."}
      action={
        <div className="flex flex-wrap justify-end gap-1.5">
          {hasPermission("query:read") && runnable.length > 0 ? (
            <Button size="sm" variant="outline" onClick={() => void runAll()} disabled={running}>
              <Play />
              {running ? "Running…" : "Run checks"}
            </Button>
          ) : null}
          {/* SEC-22 (F4): adding a rule needs governance:write, like editing and deleting one. */}
          {canWrite ? (
            <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
              <Plus />
              Add rule
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" render={<Link href="/governance/data-quality" />}>
            Open data quality
          </Button>
        </div>
      }
    >
      {a.qualityChecks.length === 0 ? (
        <EmptyState
          title="No quality checks for this asset"
          description="Add a rule to check its row count, or that a column is filled in, unique, or within a range."
          className="py-4"
        />
      ) : (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="text-xs">Check</TableHead>
                <TableHead className="text-xs">Dimension</TableHead>
                <TableHead className="text-xs">Threshold</TableHead>
                <TableHead className="text-xs">Severity</TableHead>
                <TableHead className="text-xs">Result</TableHead>
                <TableHead className="text-xs">Last run</TableHead>
                <TableHead className="text-xs" aria-label="Actions" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {a.qualityChecks.map((q) => (
                <CheckRow
                  key={q.id}
                  q={q}
                  onEdit={canWrite ? setEditing : undefined}
                  onDelete={canWrite ? setDeleting : undefined}
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => (open ? undefined : setDeleting(null))}
        title="Delete quality rule"
        description={`Delete ${deleting?.name ?? "this rule"}?`}
        impact="The rule and the results recorded for it are removed. It will no longer count toward this asset's health."
        confirmLabel="Delete rule"
        confirming={remove.status === "pending"}
        destructive
        onConfirm={() => void confirmDelete()}
      />
      {editing ? (
        <EditRuleDialog
          rule={{
            id: editing.id,
            name: editing.name,
            // An API build older than `asset` on a check: the asset's own table.
            asset: editing.asset ?? lineageKey(a),
            threshold: editing.threshold ?? "",
            severity: editing.severity ?? "medium",
          }}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null)
            onChanged()
          }}
        />
      ) : null}
      {adding ? (
        <AddRuleDialog
          asset={a}
          onClose={() => setAdding(false)}
          onCreated={() => {
            setAdding(false)
            onChanged()
          }}
        />
      ) : null}
    </SectionCard>
  )
}
