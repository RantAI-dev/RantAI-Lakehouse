"use client"

import * as React from "react"
import { Bell, CircleCheck, CircleX, Play, Plus } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import type { QueryKeys } from "@/types/data-table"
import { apiFetch } from "@/services/http"
import {
  getRuleColumns,
  type Rule,
} from "@/features/alerts/rules-columns"

type RunResult = {
  id: string
  name: string
  type: string
  fired: boolean
  value?: number
  delivered?: { ok: boolean; error?: string }
  skipped?: string
}

const OPS = [">", ">=", "<", "<=", "=="]

const RULE_QUERY_KEYS: Partial<QueryKeys> = {
  search: "rule_search",
  page: "rule_page",
  perPage: "rule_limit",
  sort: "rule_sort",
  filters: "rule_filter",
}
const AGGS = ["sum", "avg", "max", "min", "count"]

function formatRunResult(r: RunResult): string {
  if (r.skipped) {
    return `skipped: ${r.skipped}`
  }
  if (r.type === "alert") {
    const formattedVal = Math.round(r.value ?? 0).toLocaleString("id-ID")
    if (r.fired) {
      if (r.delivered?.ok) {
        return `fired (value ${formattedVal}) · sent`
      }
      const err = r.delivered?.error ?? "unknown error"
      return `fired (value ${formattedVal}) · send failed: ${err}`
    }
    return `ok (value ${formattedVal}, not breached)`
  }
  if (r.delivered?.ok) {
    return "digest sent"
  }
  const err = r.delivered?.error ?? "unknown error"
  return `send failed: ${err}`
}

function RunResultIcon({ r }: { readonly r: RunResult }) {
  if (r.skipped) {
    return <CircleX className="size-4 text-amber-500" />
  }
  if (!r.fired) {
    return <span className="size-4 text-center text-muted-foreground">–</span>
  }
  if (r.delivered?.ok) {
    return <CircleCheck className="size-4 text-emerald-500" />
  }
  return <CircleX className="size-4 text-destructive" />
}

type AlertConditionProps = {
  readonly f: Rule
  readonly setF: React.Dispatch<React.SetStateAction<Rule>>
  readonly marts: string[]
  readonly fields: string[]
  readonly loadFields: (mart?: string) => Promise<void>
}

function AlertConditionFields({
  f,
  setF,
  marts,
  fields,
  loadFields,
}: AlertConditionProps) {
  const measurePlaceholder = fields.length > 0 ? "pick" : "pick mart first"

  return (
    <>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div className="grid gap-1.5">
          <Label>Mart (Gold)</Label>
          <Select
            value={f.mart ?? ""}
            onValueChange={(v) => {
              const mart = v ?? ""
              setF((prev) => ({ ...prev, mart, measure: "" }))
              void loadFields(mart)
            }}
          >
            <SelectTrigger>
              <SelectValue placeholder="pick a mart" />
            </SelectTrigger>
            <SelectContent>
              {marts.map((m) => (
                <SelectItem key={m} value={m}>
                  {m}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid gap-1.5">
          <Label>Measure</Label>
          <Select
            value={f.measure ?? ""}
            onValueChange={(v) => setF((prev) => ({ ...prev, measure: v ?? "" }))}
            disabled={fields.length === 0}
          >
            <SelectTrigger>
              <SelectValue placeholder={measurePlaceholder} />
            </SelectTrigger>
            <SelectContent>
              {fields.map((m) => (
                <SelectItem key={m} value={m}>
                  {m}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div className="grid gap-1.5">
          <Label>Aggregate</Label>
          <Select
            value={f.agg ?? "sum"}
            onValueChange={(v) => setF((prev) => ({ ...prev, agg: v ?? "sum" }))}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {AGGS.map((a) => (
                <SelectItem key={a} value={a}>
                  {a}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid gap-1.5">
          <Label>Operator</Label>
          <Select
            value={f.op ?? ">"}
            onValueChange={(v) => setF((prev) => ({ ...prev, op: v ?? ">" }))}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {OPS.map((o) => (
                <SelectItem key={o} value={o}>
                  {o}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid gap-1.5">
          <Label>Threshold</Label>
          <Input
            type="number"
            value={f.threshold ?? 0}
            onChange={(e) =>
              setF((prev) => ({ ...prev, threshold: Number(e.target.value) }))
            }
          />
        </div>
      </div>
    </>
  )
}

type DigestConditionProps = {
  readonly f: Rule
  readonly setF: React.Dispatch<React.SetStateAction<Rule>>
  readonly boards: { id: string; name: string }[]
}

function DigestConditionFields({ f, setF, boards }: DigestConditionProps) {
  return (
    <div className="grid gap-1.5">
      <Label>Dashboard</Label>
      <Select
        value={f.board ?? ""}
        onValueChange={(v) => setF((prev) => ({ ...prev, board: v ?? "" }))}
      >
        <SelectTrigger>
          <SelectValue placeholder="pick a dashboard" />
        </SelectTrigger>
        <SelectContent>
          {boards.map((b) => (
            <SelectItem key={b.id} value={b.id}>
              {b.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

type RuleDialogProps = {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly edit: Rule | null
  readonly f: Rule
  readonly setF: React.Dispatch<React.SetStateAction<Rule>>
  readonly marts: string[]
  readonly fields: string[]
  readonly boards: { id: string; name: string }[]
  readonly loadFields: (mart?: string) => Promise<void>
  readonly onSave: () => Promise<void>
  readonly busy: boolean
  readonly err: string | null
}

function RuleEditDialog({
  open,
  onOpenChange,
  edit,
  f,
  setF,
  marts,
  fields,
  boards,
  loadFields,
  onSave,
  busy,
  err,
}: RuleDialogProps) {
  const dialogTitle = edit ? "Edit rule" : "New rule"
  const targetPlaceholder =
    f.channel === "email"
      ? "boss@company.com"
      : "https://hooks.slack.com/services/…"

  let saveLabel = "Create"
  if (edit) {
    saveLabel = "Save"
  }
  if (busy) {
    saveLabel = "Saving…"
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{dialogTitle}</DialogTitle>
          <DialogDescription>
            Alerts fire when a Gold metric crosses a threshold. Digests summarise a
            dashboard on a schedule.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label>Name</Label>
            <Input
              value={f.name}
              onChange={(e) => setF((prev) => ({ ...prev, name: e.target.value }))}
              placeholder="e.g. Foreign visitors dropped"
            />
          </div>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label>Type</Label>
              <Select
                value={f.type}
                onValueChange={(v) =>
                  setF((prev) => ({ ...prev, type: v as "alert" | "digest" }))
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="alert">Threshold alert</SelectItem>
                  <SelectItem value="digest">Dashboard digest</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Delivery</Label>
              <Select
                value={f.channel}
                onValueChange={(v) =>
                  setF((prev) => ({ ...prev, channel: v as "webhook" | "email" }))
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="webhook">Webhook (Slack/Discord)</SelectItem>
                  <SelectItem value="email">Email (SMTP)</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          {f.type === "alert" ? (
            <AlertConditionFields
              f={f}
              setF={setF}
              marts={marts}
              fields={fields}
              loadFields={loadFields}
            />
          ) : (
            <DigestConditionFields f={f} setF={setF} boards={boards} />
          )}

          <div className="grid gap-1.5">
            <Label>
              {f.channel === "email" ? "Recipient email" : "Webhook URL"}
            </Label>
            <Input
              value={f.target}
              onChange={(e) => setF((prev) => ({ ...prev, target: e.target.value }))}
              placeholder={targetPlaceholder}
            />
          </div>

          {err ? <p className="text-sm text-destructive">{err}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>
            Cancel
          </DialogClose>
          <Button size="sm" onClick={() => void onSave()} disabled={busy}>
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export function AlertRulesPage() {
  const [rules, setRules] = React.useState<Rule[]>([])
  const [marts, setMarts] = React.useState<string[]>([])
  const [boards, setBoards] = React.useState<{ id: string; name: string }[]>([])
  const [fields, setFields] = React.useState<string[]>([])
  const [open, setOpen] = React.useState(false)
  const [busy, setBusy] = React.useState(false)
  const [err, setErr] = React.useState<string | null>(null)
  const [results, setResults] = React.useState<RunResult[] | null>(null)
  const [edit, setEdit] = React.useState<Rule | null>(null)

  const [f, setF] = React.useState<Rule>({
    id: "",
    name: "",
    type: "alert",
    agg: "sum",
    op: ">",
    threshold: 0,
    channel: "webhook",
    target: "",
    enabled: true,
  })

  const load = React.useCallback(async () => {
    const [r, b] = await Promise.all([
      apiFetch("/api/alerts", { cache: "no-store" }).then((x) => x.json()),
      apiFetch("/api/dashboard/boards", { cache: "no-store" }).then((x) => x.json()),
    ])
    setRules(r.rules ?? [])
    setBoards((b.boards ?? []).filter((x: { id: string }) => x.id !== "default"))
    apiFetch("/api/dashboard/fields")
      .then((x) => x.json())
      .then((j) => setMarts((j.marts ?? []).map((m: { name: string }) => m.name)))
      .catch(() => {})
  }, [])

  React.useEffect(() => {
    void load()
  }, [load])

  const loadFields = React.useCallback(async (mart = "") => {
    if (!mart) {
      setFields([])
      return
    }
    const j = await apiFetch(`/api/dashboard/fields?mart=${encodeURIComponent(mart)}`).then(
      (x) => x.json()
    )
    setFields(j.measures ?? [])
  }, [])

  function openNew() {
    setEdit(null)
    setF({
      id: "",
      name: "",
      type: "alert",
      mart: "",
      measure: "",
      agg: "sum",
      op: ">",
      threshold: 0,
      board: boards[0]?.id,
      channel: "webhook",
      target: "",
      enabled: true,
    })
    setFields([])
    setErr(null)
    setOpen(true)
  }

  function openEdit(r: Rule) {
    setEdit(r)
    setF({ ...r })
    setErr(null)
    setOpen(true)
    if (r.mart) void loadFields(r.mart)
  }

  async function save() {
    setErr(null)
    if (!f.name.trim()) {
      setErr("Name required.")
      return
    }
    if (!f.target.trim()) {
      setErr("Webhook URL / email required.")
      return
    }
    setBusy(true)
    try {
      const res = await apiFetch("/api/alerts", {
        method: edit ? "PUT" : "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(edit ? { ...f, id: edit.id } : f),
      })
      const j = await res.json()
      if (!res.ok) throw new Error(j?.error ?? "failed")
      setOpen(false)
      await load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  async function remove(id: string) {
    await apiFetch(`/api/alerts?id=${encodeURIComponent(id)}`, { method: "DELETE" })
    await load()
  }

  async function toggle(r: Rule) {
    await apiFetch("/api/alerts", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ...r, enabled: !r.enabled }),
    })
    await load()
  }

  async function run(only?: string) {
    setBusy(true)
    setResults(null)
    try {
      const q = only ? `?id=${encodeURIComponent(only)}` : ""
      const j = await apiFetch(`/api/alerts/run${q}`, { method: "POST" }).then((x) =>
        x.json()
      )
      setResults(j.results ?? [])
    } finally {
      setBusy(false)
    }
  }

  const columns = React.useMemo(
    () =>
      getRuleColumns({
        onEdit: openEdit,
        onToggle: (r) => void toggle(r),
        onRun: (id) => void run(id),
        onDelete: (id) => void remove(id),
        busy,
        boards,
      }),
    [busy, boards]
  )

  const tableUrlState = useTableUrlState(RULE_QUERY_KEYS)
  const filteredRules = React.useMemo(
    () =>
      filterDataClientSide(rules, {
        search: tableUrlState.search,
        searchFields: [(r) => r.name, (r) => r.mart, (r) => r.measure, (r) => r.target],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [rules, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredRules,
    columns,
    queryKeys: RULE_QUERY_KEYS,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/alerts/rules",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="flex items-center gap-2 text-xl font-semibold">
            <Bell className="size-5" /> Alert Rules &amp; Digests
          </h2>
          <p className="text-sm text-muted-foreground">
            Threshold alerts on Gold metrics + scheduled dashboard digests. Delivered via
            webhook or email.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => void run()}
            disabled={busy}
          >
            <Play className="size-4" /> Run all now
          </Button>
          <Button size="sm" onClick={openNew}>
            <Plus className="size-4" /> New rule
          </Button>
        </div>
      </div>

      {results ? (
        <div className="rounded-lg border border-border bg-card/50 p-3 text-sm">
          <p className="mb-2 font-medium">Run results ({results.length})</p>
          <ul className="space-y-1">
            {results.map((r) => (
              <li key={r.id} className="flex items-center gap-2">
                <RunResultIcon r={r} />
                <span className="font-medium">{r.name}</span>
                <span className="text-muted-foreground">{formatRunResult(r)}</span>
              </li>
            ))}
            {results.length === 0 ? (
              <li className="text-muted-foreground">No enabled rules.</li>
            ) : null}
          </ul>
        </div>
      ) : null}

      <div className="space-y-4">
        <DataTableAdvancedToolbar table={table}>
          <DataTableSearch
            placeholder="Search rules..."
          />
        </DataTableAdvancedToolbar>
        <div className="rounded-md border">
          <DataTable table={table} />
        </div>
      </div>

      <p className="text-xs text-muted-foreground">
        Scheduling: hit{" "}
        <code className="rounded bg-muted px-1 font-mono">POST /api/alerts/run</code>{" "}
        periodically from cron (e.g. every 15 min). Email needs{" "}
        <code className="rounded bg-muted px-1 font-mono">
          SMTP_HOST/PORT/USER/PASS/FROM
        </code>{" "}
        env; webhook works out of the box.
      </p>

      <RuleEditDialog
        open={open}
        onOpenChange={setOpen}
        edit={edit}
        f={f}
        setF={setF}
        marts={marts}
        fields={fields}
        boards={boards}
        loadFields={loadFields}
        onSave={save}
        busy={busy}
        err={err}
      />
    </div>
  )
}
export { AlertRulesPage as AlertsPage }

