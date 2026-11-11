"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
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
import Link from "next/link"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { alertRuleService, connectorService, dashboardService, metricService } from "@/services"
import {
  LOAD_ALERT_KINDS,
  QUALITY_ALERT_KIND,
  type AlertRule,
  type SaveAlertRuleInput,
} from "@/services/contracts/alerts"
import { isLoadAlertKind, isQualityAlertKind, ruleKindLabel } from "./rules-columns"
import type { Metric } from "@/services/contracts/metrics"

/**
 * `/alerts/rules/new` and `/alerts/rules/[id]/edit` (`WEB-7` T7, D2/D4/D7):
 * the alert rule form as its own page.
 *
 * `RuleEditDialog` was the last pop-up of the alerts list. The form itself is
 * moved here unchanged: `alertRuleFormFields` and `alertRuleSaveInput` stay
 * pure functions and are reused by the list, which still has to write a rule
 * back when it is switched on or off.
 *
 * Edit loads the rule through `alertRuleService.listRules` (there is no
 * by-id read), so a reload works; an id that names no rule shows the
 * not-found state. The boards and the marts come from `dashboardService`
 * now, which is the layering fix D7 asks for: no feature file calls
 * `/api/dashboard/boards` or `/api/dashboard/fields` by hand.
 */

const OPS = [">", ">=", "<", "<=", "=="]
const AGGS = ["sum", "avg", "max", "min", "count"]

const EMPTY_FORM: AlertRule = {
  id: "", name: "", type: "alert", agg: "sum", op: ">", threshold: 0, channel: "webhook", target: "", enabled: true,
};

/**
 * D5, corrected by `SRC-17` D-c: the line under the two kinds with something
 * special to say. `connector_disabled` is raised since `SRC-11`: once, when a
 * connector fails runs in a row and is paused (30 by default, a setting of the
 * API). `connector_schema_change` is raised: schema change
 * detection (`SRC-8`) sends it when a source table's columns change or a new
 * table appears, so the page no longer tells the user a working rule does not
 * work.
 */
const KIND_NOTES: Record<string, string> = {
  connector_disabled:
    "Raised once when a connector fails runs in a row (30 by default) and is paused. Resume it from its page.",
  connector_schema_change:
    "Raised when a source table's columns change or a new table appears at the source.",
  connector_lag:
    "Raised once when a change-capture connector falls further behind the source than its limit (set on the connector's Change capture tab, 15 minutes by default), and once more when it is back under.",
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

/** BI-2: a threshold rule that watches a metric (an empty name is a metric rule not yet filled in). */
export function isMetricRule(rule: { metric?: string }): boolean {
  return rule.metric !== undefined
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
 * line under the kinds in `KIND_NOTES` (D5, `SRC-17` D-c). Kept pure and DOM-free so it
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
  // `DATA-14`: a quality incident rule has a table (`asset`), nothing else; the
  // table field is shown for it by `isQualityAlertKind`, so this return keeps
  // its shape.
  if (isQualityAlertKind(type)) return { martMeasure: false, board: false, freshnessTarget: false, ...none };
  if (type === "pipeline_behind_target") return { martMeasure: false, board: false, freshnessTarget: false, ...none };
  if (type === "alert") return { martMeasure: true, board: false, freshnessTarget: false, ...none };
  if (isLoadAlertKind(type)) {
    const picksConnector = type !== "upload_failure"
    return {
      martMeasure: false,
      board: false,
      freshnessTarget: false,
      connector: picksConnector,
      allConnectors: picksConnector && type !== "connector_success" && canSeeAllTenants,
      note: KIND_NOTES[type] ?? null,
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
  if (isQualityAlertKind(f.type)) {
    return {
      name: f.name.trim(),
      type: f.type,
      channel: f.channel,
      target: f.target.trim(),
      enabled: f.enabled,
      ...(f.severity ? { severity: f.severity } : {}),
      asset: (f.asset ?? "").trim(),
    }
  }
  if (f.type === "pipeline_behind_target") {
    return {
      name: f.name.trim(),
      type: f.type,
      channel: f.channel,
      target: f.target.trim(),
      enabled: f.enabled,
      pipeline: f.pipeline?.trim() || "*",
      ...(f.severity ? { severity: f.severity } : {}),
    }
  }
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

type AlertConditionProps = {
  readonly f: AlertRule
  readonly setF: React.Dispatch<React.SetStateAction<AlertRule>>
  readonly marts: string[]
  readonly martsError: string | null
  readonly fields: string[]
  readonly fieldsError: string | null
  readonly loadFields: (mart: string) => Promise<void>
  /** BI-2: the saved metrics a rule can watch. */
  readonly metrics: Metric[]
}

const WATCH_ITEMS = { column: "A column of a table", metric: "A metric" }

function AlertConditionFields({
  f,
  setF,
  marts,
  martsError,
  fields,
  fieldsError,
  loadFields,
  metrics,
}: AlertConditionProps) {
  const measurePlaceholder = fields.length > 0 ? "pick" : "pick mart first"
  const watching = isMetricRule(f) ? "metric" : "column"

  return (
    <>
      <div className="grid gap-1.5">
        <Label>Watch</Label>
        <Select
          value={watching}
          items={WATCH_ITEMS}
          onValueChange={(v) =>
            setF((prev) =>
              v === "metric" ? { ...prev, metric: prev.metric ?? "", mart: undefined, measure: undefined } : { ...prev, metric: undefined },
            )
          }
        >
          <SelectTrigger className="w-full" aria-label="Watch">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="column">{WATCH_ITEMS.column}</SelectItem>
            <SelectItem value="metric">{WATCH_ITEMS.metric}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {watching === "metric" ? (
        <div className="grid gap-1.5">
          <Label>Metric</Label>
          <Select
            value={f.metric ?? ""}
            onValueChange={(v) => setF((prev) => ({ ...prev, metric: v ?? "" }))}
          >
            <SelectTrigger className="w-full" aria-label="Metric">
              <SelectValue placeholder={metrics.length ? "pick a metric" : "no metrics yet"} />
            </SelectTrigger>
            <SelectContent>
              {metrics.map((m) => (
                <SelectItem key={m.id} value={m.name}>
                  {m.label || m.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      ) : null}
      {watching === "metric" ? null : (
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
            <SelectTrigger className="w-full">
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
            <SelectTrigger className="w-full">
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
      )}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {watching === "metric" ? null : (
        <div className="grid gap-1.5">
          <Label>Aggregate</Label>
          <Select
            value={f.agg ?? "sum"}
            onValueChange={(v) => setF((prev) => ({ ...prev, agg: v ?? "sum" }))}
          >
            <SelectTrigger className="w-full">
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
        )}
        <div className="grid gap-1.5">
          <Label>Operator</Label>
          <Select
            value={f.op ?? ">"}
            onValueChange={(v) => setF((prev) => ({ ...prev, op: v ?? ">" }))}
          >
            <SelectTrigger className="w-full">
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
        <SelectTrigger className="w-full">
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

/**
 * `BI-39` D4: a rule saved before alerts had owners still reads as the Dashboard
 * Viewer role, and the editor says so — saving it makes it read as the person
 * saving. `null` for a digest, an event rule, a new rule, or an owned one.
 */
export function legacyOwnerNote(
  rule: Pick<AlertRule, "type" | "ownerId"> | null
): string | null {
  if (!rule || rule.type !== "alert" || rule.ownerId) return null
  return "This alert was set before alerts had owners and reads as the Dashboard Viewer role. Saving it makes it read as you."
}

/** The stored rule these fields show, or `null` for a new one. */
export function AlertRulePage({ ruleId }: { ruleId?: string }) {
  const router = useRouter()
  const backHref = "/alerts"
  const { user } = useAuth()
  const canSeeAllTenants = seesEveryTenant(user?.permissions)

  // The rule being edited, read from the list the page already has to ask
  // for (there is no by-id read). A new rule asks for nothing.
  const rules = useService(
    (signal) => (ruleId ? alertRuleService.listRules(signal) : Promise.resolve([] as AlertRule[])),
    [ruleId]
  )
  const boardsState = useService((signal) => dashboardService.listBoards(signal), [])
  const martsState = useService((signal) => dashboardService.listMarts(signal), [])
  const connectorList = useService((signal) => connectorService.listConnectors(signal), [])
  const metricsState = useService((signal) => metricService.list(signal), [])

  const [f, setF] = React.useState<AlertRule>(EMPTY_FORM)
  const [fields, setFields] = React.useState<string[]>([])
  const [fieldsError, setFieldsError] = React.useState<string | null>(null)
  const [formErr, setFormErr] = React.useState<string | null>(null)
  // The saved rule the form was filled from: only it knows whether it was
  // ever given an owner, and a rename is not a save of what the form holds.
  const [stored, setStored] = React.useState<AlertRule | null>(null)
  const seeded = React.useRef(false)

  const saveAction = useServiceAction(
    (signal, input: SaveAlertRuleInput, id: string | null) =>
      id ? alertRuleService.updateRule({ ...input, id }, signal) : alertRuleService.createRule(input, signal)
  )

  async function loadFields(mart: string) {
    setFieldsError(null)
    if (!mart) {
      setFields([])
      return
    }
    try {
      setFields(await dashboardService.listMartMeasures(mart))
    } catch (e) {
      setFieldsError(e instanceof Error ? e.message : "Fields could not be loaded")
      setFields([])
    }
  }

  const rule = ruleId
    ? rules.status === "success"
      ? (rules.data.find((candidate) => candidate.id === ruleId) ?? null)
      : undefined
    : null

  React.useEffect(() => {
    if (!rule || seeded.current) return
    seeded.current = true
    setStored(rule)
    setF({ ...rule })
    // The rows this rule watches are the mart's, so the measure picker is
    // given them before the form is used.
    if (rule.mart && alertRuleFormFields(rule.type, canSeeAllTenants).martMeasure) {
      void loadFields(rule.mart)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- seed once, from the rule the read found
  }, [rule])

  if (ruleId) {
    if (rules.status === "loading") return <LoadingSkeleton rows={6} />
    if (rules.status === "error") return <ErrorState error={rules.error} onRetry={rules.reload} />
    // One frame between the read arriving and the form being filled from it:
    // a skeleton, never the not-found state that would flash on every visit.
    if (rule && !seeded.current) return <LoadingSkeleton rows={6} />
    if (!rule) {
      return (
        <EmptyState
          title="Rule not found"
          description="This rule was deleted, or the address names one that never existed."
          action={
            <Button variant="outline" size="sm" render={<Link href={backHref} />}>
              Back to Alerts
            </Button>
          }
        />
      )
    }
  }

  const targetPlaceholder =
    f.channel === "email" ? "boss@company.com" : "https://hooks.slack.com/services/…"
  const formFields = alertRuleFormFields(f.type, canSeeAllTenants)
  const ownerNote = legacyOwnerNote(stored)
  const connectors =
    connectorList.status === "success"
      ? connectorList.data.map((c) => ({ id: c.id, name: c.name }))
      : []
  const connectorsError =
    connectorList.status === "error" ? connectorList.error.message : null
  const marts = martsState.status === "success" ? martsState.data.map((m) => m.name) : []
  const martsError = martsState.status === "error" ? martsState.error.message : null
  const boards =
    boardsState.status === "success"
      ? boardsState.data
          .filter((b) => b.id !== "default")
          .map((b) => ({ id: b.id, name: b.name }))
      : []
  const boardsError = boardsState.status === "error" ? boardsState.error.message : null
  const metrics = metricsState.status === "success" ? metricsState.data : []

  async function save() {
    setFormErr(null)
    if (!f.name.trim()) { setFormErr("Name required."); return }
    if (!f.target.trim()) { setFormErr("Webhook URL / email required."); return }
    if (f.type === "alert" && isMetricRule(f) && !f.metric) { setFormErr("Pick a metric."); return }
    if (alertRuleFormFields(f.type, canSeeAllTenants).connector && !f.connector) {
      setFormErr("Choose a connector.")
      return
    }
    if (isQualityAlertKind(f.type) && !(f.asset ?? "").trim()) {
      setFormErr("Name the table, or * for every table.")
      return
    }
    const saved = await saveAction.run(alertRuleSaveInput(f), ruleId ?? null)
    if (saved) router.push(backHref)
  }

  return (
    <FormPage
      title={ruleId ? "Edit rule" : "New rule"}
      description="Alerts fire when a Gold metric crosses a threshold. Digests summarise a dashboard on a schedule. Connector and upload rules tell you when a load fails or succeeds."
      backHref={backHref}
      backLabel="Back to Alerts"
      submitLabel={ruleId ? "Save" : "Create"}
      submittingLabel={ruleId ? "Saving…" : "Creating…"}
      // The form checks on submit, as the dialog did: every message names the
      // field to fill in.
      canSubmit
      submitting={saveAction.status === "pending"}
      error={formErr ?? saveAction.error?.message ?? null}
      onSubmit={() => void save()}
      onCancel={() => router.push(backHref)}
      className="max-w-xl"
    >
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
                <SelectTrigger className="w-full">
                  <SelectValue>{ruleKindLabel(f.type)}</SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="alert">Threshold alert</SelectItem>
                  <SelectItem value="digest">Dashboard digest</SelectItem>
                  <SelectItem value="freshness">Dataset freshness</SelectItem>
                  <SelectItem value="pipeline_behind_target">Pipeline behind target lag</SelectItem>
                  {LOAD_ALERT_KINDS.map((kind) => (
                    <SelectItem key={kind} value={kind}>
                      {ruleKindLabel(kind)}
                    </SelectItem>
                  ))}
                  {/* DATA-14: only an administrator who sees every tenant may save
                      one (the API enforces it); the form offers what will save. */}
                  {canSeeAllTenants || isQualityAlertKind(f.type) ? (
                    <SelectItem value={QUALITY_ALERT_KIND}>{ruleKindLabel(QUALITY_ALERT_KIND)}</SelectItem>
                  ) : null}
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
                <SelectTrigger className="w-full">
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
              <SelectTrigger className="w-full">
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

          {isQualityAlertKind(f.type) ? (
            <div className="grid gap-1.5">
              <Label>Table (zone.table, or * for every table)</Label>
              <Input
                value={f.asset ?? ""}
                onChange={(e) => setF((prev) => ({ ...prev, asset: e.target.value }))}
                placeholder="silver.orders"
              />
              <p className="text-xs text-muted-foreground">
                Sent when a quality incident opens on the table: a check that fails, or a load far
                smaller or later than usual. An incident that is still open is not sent again.
              </p>
            </div>
          ) : null}

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
              metrics={metrics}
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

          {ownerNote ? (
            <p className="text-xs text-muted-foreground">{ownerNote}</p>
          ) : null}
        </div>
    </FormPage>
  )
}
