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
// `/api/dashboard/boards` and `/api/dashboard/fields` only populate this
// dialog's board/mart/measure selects; every dashboard feature fetches them
// the same way (dashboard-page.tsx, chart-builder.tsx, dashboard-filters.tsx)
// because there is no dashboard client yet (a pre-existing layering gap,
// out of scope for WS1 task 1.15) — so these two lookups stay page-local
// instead of going through `@/services`.
import { apiFetch } from "@/services/http"
import { useAuth } from "@/features/auth/auth-provider"
import { getRuleColumns, isLoadAlertKind, ruleKindLabel, type Rule } from "@/features/alerts/rules-columns"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService, useServiceAction } from "@/hooks/use-service"
import { alertRuleService, connectorService } from "@/services"
import {
  LOAD_ALERT_KINDS,
  type AlertRule,
  type SaveAlertRuleInput,
  type AlertRunResult,
} from "@/services/contracts/alerts"

const OPS = [">", ">=", "<", "<=", "=="]
const AGGS = ["sum", "avg", "max", "min", "count"]

const RULE_QUERY_KEYS: Partial<QueryKeys> = {
  search: "rule_search",
  page: "rule_page",
  perPage: "rule_limit",
  sort: "rule_sort",
  filters: "rule_filter",
}

function formatRunResult(r: AlertRunResult): string {
  if (r.skipped) {
    return `skipped: ${r.skipped}`
  }
  if (r.type === "alert") {
    const formattedVal = Math.round(r.value ?? 0).toLocaleString()
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

function RunResultIcon({ r }: { readonly r: AlertRunResult }) {
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

const EMPTY_FORM: AlertRule = {
  id: "", name: "", type: "alert", agg: "sum", op: ">", threshold: 0, channel: "webhook", target: "", enabled: true,
};

/** D5: shown under the two kinds that nothing raises yet. */
const NOT_RAISED_YET: Record<string, string> = {
  connector_disabled:
    "Nothing raises this yet. It starts working when automatic pausing (SRC-11) is installed.",
  connector_schema_change:
    "Nothing raises this yet. It starts working when schema change detection (SRC-8) is installed.",
}

/**
 * Whether the signed-in user sees every tenant: the one grant the API's
 * `is_unrestricted` checks, the literal `*:*` token (a Platform Admin's set).
 * Only such a user may save a rule for all connectors or for uploads (`SRC-7`
 * D7); the API enforces it, this only decides what the form offers.
 */
export function seesEveryTenant(permissions: readonly string[] | undefined): boolean {
  return permissions?.includes("*:*") ?? false
}

/**
 * Which rule-editor fields apply to a given `AlertRule.type`. A
 * `freshness` rule reuses `mart` as its `<namespace>.<table>` target and
 * sends no `board` — the backend clears `measure`/`agg`/`threshold`/`board`
 * for that kind (`lakehouse-alerts::normalize_input`). The six `SRC-7` kinds
 * have none of mart, measure, aggregate, operator, threshold or board: the
 * five `connector_*` kinds pick a connector (`connector`), offering "All
 * connectors" (`allConnectors`) only when `canSeeAllTenants` and never for
 * `connector_success` (D4); `upload_failure` picks nothing. `note` is the
 * line under the kinds nothing raises yet (D5). Kept pure and DOM-free so it
 * is unit-testable (WS5 item E3).
 */
export function alertRuleFormFields(
  type: string,
  canSeeAllTenants = false
): {
  martMeasure: boolean;
  board: boolean;
  freshnessTarget: boolean;
  connector: boolean;
  allConnectors: boolean;
  note: string | null;
} {
  const none = { connector: false, allConnectors: false, note: null }
  if (type === "freshness") return { martMeasure: false, board: false, freshnessTarget: true, ...none };
  if (type === "alert") return { martMeasure: true, board: false, freshnessTarget: false, ...none };
  if (isLoadAlertKind(type)) {
    const picksConnector = type !== "upload_failure"
    return {
      martMeasure: false,
      board: false,
      freshnessTarget: false,
      connector: picksConnector,
      allConnectors: picksConnector && type !== "connector_success" && canSeeAllTenants,
      note: NOT_RAISED_YET[type] ?? null,
    }
  }
  return { martMeasure: false, board: true, freshnessTarget: false, ...none };
}

/**
 * The body to save for the form `f`. The `SRC-7` kinds send exactly the fields
 * that apply (no mart, measure, aggregate, operator, threshold, board or
 * pipeline) and only a connector for the five connector kinds; every other
 * kind sends the form as it always did.
 */
export function alertRuleSaveInput(f: AlertRule): SaveAlertRuleInput {
  if (!isLoadAlertKind(f.type)) return f
  return {
    name: f.name.trim(),
    type: f.type,
    channel: f.channel,
    target: f.target.trim(),
    enabled: f.enabled,
    ...(f.severity ? { severity: f.severity } : {}),
    ...(f.type === "upload_failure" ? {} : { connector: f.connector ?? "" }),
  }
}

/** The connector to keep when the kind changes: "All connectors" does not survive a kind that cannot take it. */
export function connectorForKind(type: string, current: string | undefined, canSeeAllTenants: boolean): string | undefined {
  if (!alertRuleFormFields(type, canSeeAllTenants).connector) return undefined
  if (current === "*" && !alertRuleFormFields(type, canSeeAllTenants).allConnectors) return undefined
  return current
}

/** The run-alerts notice (D9), or `null` when the API says reports can arrive or could not say. */
export const RUN_ALERTS_OFF =
  "Run alerts are not being reported: PIPELINE_RUN_TOKEN is not set on the API."

export function RunAlertsNotice({ runEventsConfigured }: { readonly runEventsConfigured: boolean | null }) {
  if (runEventsConfigured !== false) return null
  return (
    <div
      role="status"
      className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
    >
      {RUN_ALERTS_OFF}
    </div>
  )
}

type AlertConditionProps = {
  readonly f: AlertRule
  readonly setF: React.Dispatch<React.SetStateAction<AlertRule>>
  readonly marts: string[]
  readonly martsError: string | null
  readonly fields: string[]
  readonly fieldsError: string | null
  readonly loadFields: (mart: string) => Promise<void>
}

function AlertConditionFields({
  f,
  setF,
  marts,
  martsError,
  fields,
  fieldsError,
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
          {martsError ? <p className="mt-1 text-xs text-destructive">{martsError}</p> : null}
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
          {fieldsError ? <p className="mt-1 text-xs text-destructive">{fieldsError}</p> : null}
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
  readonly f: AlertRule
  readonly setF: React.Dispatch<React.SetStateAction<AlertRule>>
  readonly boards: { id: string; name: string }[]
  readonly boardsError: string | null
}

function DigestConditionFields({ f, setF, boards, boardsError }: DigestConditionProps) {
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
      {boardsError ? <p className="mt-1 text-xs text-destructive">{boardsError}</p> : null}
    </div>
  )
}

type RuleDialogProps = {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly edit: AlertRule | null
  readonly f: AlertRule
  readonly setF: React.Dispatch<React.SetStateAction<AlertRule>>
  readonly marts: string[]
  readonly martsError: string | null
  readonly fields: string[]
  readonly fieldsError: string | null
  readonly boards: { id: string; name: string }[]
  readonly boardsError: string | null
  readonly loadFields: (mart: string) => Promise<void>
  readonly onSave: () => Promise<void>
  readonly busy: boolean
  readonly err: string | null
  readonly connectors: { id: string; name: string }[]
  readonly connectorsError: string | null
  readonly canSeeAllTenants: boolean
}

function RuleEditDialog({
  open,
  onOpenChange,
  edit,
  f,
  setF,
  marts,
  martsError,
  fields,
  fieldsError,
  boards,
  boardsError,
  loadFields,
  onSave,
  busy,
  err,
  connectors,
  connectorsError,
  canSeeAllTenants,
}: RuleDialogProps) {
  const dialogTitle = edit ? "Edit rule" : "New rule"
  const targetPlaceholder =
    f.channel === "email"
      ? "boss@company.com"
      : "https://hooks.slack.com/services/…"
  const formFields = alertRuleFormFields(f.type, canSeeAllTenants)

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
            dashboard on a schedule. Connector and upload rules tell you when a load
            fails or succeeds.
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
                  setF((prev) => {
                    const type = v ?? "alert"
                    return { ...prev, type, connector: connectorForKind(type, prev.connector, canSeeAllTenants) }
                  })
                }
              >
                <SelectTrigger>
                  <SelectValue>{ruleKindLabel(f.type)}</SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="alert">Threshold alert</SelectItem>
                  <SelectItem value="digest">Dashboard digest</SelectItem>
                  <SelectItem value="freshness">Dataset freshness</SelectItem>
                  {LOAD_ALERT_KINDS.map((kind) => (
                    <SelectItem key={kind} value={kind}>
                      {ruleKindLabel(kind)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Delivery</Label>
              <Select
                value={f.channel}
                onValueChange={(v) =>
                  setF((prev) => ({ ...prev, channel: v ?? "webhook" }))
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

          <div className="grid gap-1.5">
            <Label>Severity</Label>
            {/* The backend's SEVERITIES accepts exactly these five strings
                and 400s on anything else; "(none)" must send no `severity`
                field at all, never an empty string or the literal "none"
                the backend would reject. */}
            <Select
              value={f.severity ?? "none"}
              onValueChange={(v) => setF((prev) => ({ ...prev, severity: !v || v === "none" ? undefined : v }))}
            >
              <SelectTrigger>
                <SelectValue placeholder="Severity" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">(none)</SelectItem>
                <SelectItem value="critical">Critical</SelectItem>
                <SelectItem value="high">High</SelectItem>
                <SelectItem value="medium">Medium</SelectItem>
                <SelectItem value="low">Low</SelectItem>
                <SelectItem value="info">Info</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {formFields.connector ? (
            <div className="grid gap-1.5">
              <Label>Connector</Label>
              <Select
                value={f.connector ?? ""}
                onValueChange={(v) => setF((prev) => ({ ...prev, connector: v ?? "" }))}
              >
                <SelectTrigger>
                  <SelectValue placeholder="pick a connector">
                    {f.connector === "*"
                      ? "All connectors"
                      : (connectors.find((c) => c.id === f.connector)?.name ?? f.connector)}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  {formFields.allConnectors ? <SelectItem value="*">All connectors</SelectItem> : null}
                  {connectors.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {connectorsError ? <p className="mt-1 text-xs text-destructive">{connectorsError}</p> : null}
            </div>
          ) : null}
          {formFields.note ? <p className="text-xs text-muted-foreground">{formFields.note}</p> : null}

          {formFields.freshnessTarget ? (
            <div className="grid gap-1.5">
              <Label>Table (namespace.table)</Label>
              <Input
                value={f.mart ?? ""}
                onChange={(e) => setF((prev) => ({ ...prev, mart: e.target.value }))}
                placeholder="bronze.orders"
              />
            </div>
          ) : formFields.martMeasure ? (
            <AlertConditionFields
              f={f}
              setF={setF}
              marts={marts}
              martsError={martsError}
              fields={fields}
              fieldsError={fieldsError}
              loadFields={loadFields}
            />
          ) : formFields.board ? (
            <DigestConditionFields f={f} setF={setF} boards={boards} boardsError={boardsError} />
          ) : null}

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

export function AlertsPage() {
  const state = useService((s) => alertRuleService.listRules(s), []);
  // D9: says whether run reports can arrive at all. If it cannot be read, no
  // notice is shown (the rules list reports its own failure).
  const runStatus = useService((s) => alertRuleService.getStatus(s), []);
  const connectorList = useService((s) => connectorService.listConnectors(s), []);
  const { user } = useAuth();
  const canSeeAllTenants = seesEveryTenant(user?.permissions);
  const connectors = React.useMemo(
    () => (connectorList.status === "success" ? connectorList.data.map((c) => ({ id: c.id, name: c.name })) : []),
    [connectorList]
  );

  const [marts, setMarts] = React.useState<string[]>([]);
  const [martsError, setMartsError] = React.useState<string | null>(null);
  const [boards, setBoards] = React.useState<{ id: string; name: string }[]>([]);
  const [boardsError, setBoardsError] = React.useState<string | null>(null);
  const [fields, setFields] = React.useState<string[]>([]);
  const [fieldsError, setFieldsError] = React.useState<string | null>(null);

  const [open, setOpen] = React.useState(false);
  const [formErr, setFormErr] = React.useState<string | null>(null);
  const [edit, setEdit] = React.useState<AlertRule | null>(null);
  const [f, setF] = React.useState<AlertRule>(EMPTY_FORM);

  const saveAction = useServiceAction(
    (signal, input: SaveAlertRuleInput, id: string | null) =>
      id ? alertRuleService.updateRule({ ...input, id }, signal) : alertRuleService.createRule(input, signal)
  );
  const removeAction = useServiceAction((signal, id: string) => alertRuleService.deleteRule(id, signal));
  const toggleAction = useServiceAction((signal, rule: AlertRule) =>
    // A load rule goes back with only the fields that apply to it, as a save does.
    alertRuleService.updateRule({ ...alertRuleSaveInput(rule), id: rule.id, enabled: !rule.enabled }, signal)
  );
  const runAction = useServiceAction((signal, id: string | undefined) => alertRuleService.runRules(id, signal));

  // Dashboard board list and mart/measure names: page-local (see import
  // comment above), but every failure is reported instead of leaving the
  // selects silently empty.
  React.useEffect(() => {
    let active = true;
    apiFetch("/api/dashboard/boards", { cache: "no-store" })
      .then(async (res) => {
        const json = await res.json();
        if (!active) return;
        if (!res.ok) { setBoardsError(json?.error ?? "Boards could not be loaded"); return; }
        setBoards((json.boards ?? []).filter((x: { id: string }) => x.id !== "default"));
      })
      .catch((e) => { if (active) setBoardsError(e instanceof Error ? e.message : "Boards could not be loaded"); });
    apiFetch("/api/dashboard/fields", { cache: "no-store" })
      .then(async (res) => {
        const json = await res.json();
        if (!active) return;
        if (!res.ok) { setMartsError(json?.error ?? "Marts could not be loaded"); return; }
        setMarts((json.marts ?? []).map((m: { name: string }) => m.name));
      })
      .catch((e) => { if (active) setMartsError(e instanceof Error ? e.message : "Marts could not be loaded"); });
    return () => { active = false; };
  }, []);

  async function loadFields(mart: string) {
    setFieldsError(null);
    if (!mart) { setFields([]); return; }
    try {
      const res = await apiFetch(`/api/dashboard/fields?mart=${encodeURIComponent(mart)}`);
      const json = await res.json();
      if (!res.ok) { setFieldsError(json?.error ?? "Fields could not be loaded"); setFields([]); return; }
      setFields(json.measures ?? []);
    } catch (e) {
      setFieldsError(e instanceof Error ? e.message : "Fields could not be loaded");
      setFields([]);
    }
  }

  function openNew() {
    setEdit(null);
    setF({ ...EMPTY_FORM, board: boards[0]?.id });
    setFields([]); setFieldsError(null); setFormErr(null); saveAction.reset(); setOpen(true);
  }
  function openEdit(r: AlertRule) {
    setEdit(r); setF({ ...r }); setFormErr(null); saveAction.reset(); setOpen(true);
    if (r.mart) void loadFields(r.mart);
  }

  async function save() {
    setFormErr(null);
    if (!f.name.trim()) { setFormErr("Name required."); return; }
    if (!f.target.trim()) { setFormErr("Webhook URL / email required."); return; }
    if (alertRuleFormFields(f.type, canSeeAllTenants).connector && !f.connector) {
      setFormErr("Choose a connector.");
      return;
    }
    const saved = await saveAction.run(alertRuleSaveInput(f), edit ? edit.id : null);
    if (saved) { setOpen(false); state.reload(); }
  }

  async function remove(id: string) {
    const result = await removeAction.run(id);
    if (result !== null) state.reload();
  }
  async function toggle(r: AlertRule) {
    const result = await toggleAction.run(r);
    if (result) state.reload();
  }

  async function run(only?: string) {
    await runAction.run(only);
  }

  const mutationError = removeAction.error ?? toggleAction.error ?? runAction.error;
  const rules = state.status === "success" ? state.data : [];
  const busy = saveAction.status === "pending";
  const runResults = runAction.status === "success" ? runAction.data.results : null;

  const columns = React.useMemo(
    () =>
      getRuleColumns({
        onEdit: openEdit,
        onToggle: (r) => void toggle(r),
        onRun: (id) => void run(id),
        onDelete: (id) => void remove(id),
        busy: toggleAction.status === "pending" || runAction.status === "pending",
        boards,
        connectors,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [toggleAction.status, runAction.status, boards, connectors]
  )

  const tableUrlState = useTableUrlState(RULE_QUERY_KEYS)
  const filteredRules = React.useMemo(
    () =>
      filterDataClientSide(
        // `getRuleColumns`/`Rule` predate the real `AlertRule` contract (it
        // has no "freshness" kind or `severity`); a real rule is a
        // structural superset of `Rule`, so this is a widening cast, not a
        // fabrication — every field `Rule` reads is genuinely present.
        rules as unknown as Rule[],
        {
          search: tableUrlState.search,
          searchFields: [(r) => r.name, (r) => r.mart, (r) => r.measure, (r) => r.target],
          filters: tableUrlState.filters,
          joinOperator: tableUrlState.joinOperator,
        }
      ),
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
          <Button variant="outline" size="sm" onClick={() => void run()} disabled={runAction.status === "pending"}>
            <Play className="size-4" /> Run all now
          </Button>
          <Button size="sm" onClick={openNew}>
            <Plus className="size-4" /> New rule
          </Button>
        </div>
      </div>

      <RunAlertsNotice
        runEventsConfigured={runStatus.status === "success" ? runStatus.data.runEventsConfigured : null}
      />

      {mutationError ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {mutationError.message}
        </div>
      ) : null}

      {runResults ? (
        <div className="rounded-lg border border-border bg-card/50 p-3 text-sm">
          <p className="mb-2 font-medium">Run results ({runResults.length})</p>
          <ul className="space-y-1">
            {runResults.map((r) => (
              <li key={r.id} className="flex items-center gap-2">
                <RunResultIcon r={r} />
                <span className="font-medium">{r.name}</span>
                <span className="text-muted-foreground">{formatRunResult(r)}</span>
              </li>
            ))}
            {runResults.length === 0 ? <li className="text-muted-foreground">No enabled rules.</li> : null}
          </ul>
        </div>
      ) : null}

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch placeholder="Search rules..." />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}

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
        martsError={martsError}
        fields={fields}
        fieldsError={fieldsError}
        boards={boards}
        boardsError={boardsError}
        loadFields={loadFields}
        onSave={save}
        busy={busy}
        err={formErr ?? saveAction.error?.message ?? null}
        connectors={connectors}
        connectorsError={connectorList.status === "error" ? connectorList.error.message : null}
        canSeeAllTenants={canSeeAllTenants}
      />
    </div>
  )
}

export { AlertsPage as AlertRulesPage }
