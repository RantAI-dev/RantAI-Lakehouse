"use client";

import * as React from "react";
import { Bell, Plus, Play, Trash2, Send, Webhook, Mail, CircleCheck, CircleX } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter, DialogClose,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
// `/api/dashboard/boards` and `/api/dashboard/fields` only populate this
// dialog's board/mart/measure selects; every dashboard feature fetches them
// the same way (dashboard-page.tsx, chart-builder.tsx, dashboard-filters.tsx)
// because there is no dashboard client yet (a pre-existing layering gap,
// out of scope for WS1 task 1.15) — so these two lookups stay page-local
// instead of going through `@/services`.
import { apiFetch } from "@/services/http";
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states";
import { useService, useServiceAction } from "@/hooks/use-service";
import { alertRuleService } from "@/services";
import type { AlertRule, SaveAlertRuleInput } from "@/services/contracts/alerts";

const OPS = [">", ">=", "<", "<=", "=="];
const AGGS = ["sum", "avg", "max", "min", "count"];

const EMPTY_FORM: AlertRule = {
  id: "", name: "", type: "alert", agg: "sum", op: ">", threshold: 0, channel: "webhook", target: "", enabled: true,
};

/**
 * Which rule-editor fields apply to a given `AlertRule.type`. A
 * `freshness` rule reuses `mart` as its `<namespace>.<table>` target and
 * sends no `board` — the backend clears `measure`/`agg`/`threshold`/`board`
 * for that kind (`lakehouse-alerts::normalize_input`). Kept pure and
 * DOM-free so it is unit-testable (WS5 item E3).
 */
export function alertRuleFormFields(type: string): {
  martMeasure: boolean;
  board: boolean;
  freshnessTarget: boolean;
} {
  if (type === "freshness") return { martMeasure: false, board: false, freshnessTarget: true };
  if (type === "alert") return { martMeasure: true, board: false, freshnessTarget: false };
  return { martMeasure: false, board: true, freshnessTarget: false };
}

export function AlertsPage() {
  const state = useService((s) => alertRuleService.listRules(s), []);

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
    alertRuleService.updateRule({ ...rule, enabled: !rule.enabled }, signal)
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
    const saved = await saveAction.run(f, edit ? edit.id : null);
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
  const formFields = alertRuleFormFields(f.type);
  const runResults = runAction.status === "success" ? runAction.data.results : null;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-xl font-semibold"><Bell className="size-5" /> Alerts &amp; Digests</h1>
          <p className="text-sm text-muted-foreground">Threshold alerts on Gold metrics + scheduled dashboard digests. Delivered via webhook (Slack/Discord) or email.</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => void run()} disabled={runAction.status === "pending"}><Play className="size-4" /> Run all now</Button>
          <Button size="sm" onClick={openNew}><Plus className="size-4" /> New rule</Button>
        </div>
      </div>

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
                {r.skipped ? <CircleX className="size-4 text-amber-500" /> : r.fired ? (r.delivered?.ok ? <CircleCheck className="size-4 text-emerald-500" /> : <CircleX className="size-4 text-destructive" />) : <span className="size-4 text-center text-muted-foreground">–</span>}
                <span className="font-medium">{r.name}</span>
                <span className="text-muted-foreground">
                  {r.skipped ? `skipped: ${r.skipped}` : r.type === "alert"
                    ? (r.fired ? `fired (value ${Math.round(r.value ?? 0).toLocaleString("id-ID")})${r.delivered?.ok ? " · sent" : ` · send failed: ${r.delivered?.error}`}` : `ok (value ${Math.round(r.value ?? 0).toLocaleString("id-ID")}, not breached)`)
                    : (r.delivered?.ok ? "digest sent" : `send failed: ${r.delivered?.error}`)}
                </span>
              </li>
            ))}
            {runResults.length === 0 ? <li className="text-muted-foreground">No enabled rules.</li> : null}
          </ul>
        </div>
      ) : null}

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <table className="w-full text-sm">
            <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
              <tr>
                <th className="px-3 py-2 font-medium">Name</th>
                <th className="px-3 py-2 font-medium">Type</th>
                <th className="px-3 py-2 font-medium">Condition / board</th>
                <th className="px-3 py-2 font-medium">Delivery</th>
                <th className="px-3 py-2 font-medium">On</th>
                <th className="px-3 py-2" />
              </tr>
            </thead>
            <tbody>
              {rules.map((r) => (
                <tr key={r.id} className="border-t border-border hover:bg-muted/20">
                  <td className="cursor-pointer px-3 py-2 font-medium" onClick={() => openEdit(r)}>{r.name}</td>
                  <td className="px-3 py-2"><span className={cn("rounded px-1.5 py-0.5 text-[11px]", r.type === "alert" ? "bg-amber-500/10 text-amber-600 dark:text-amber-400" : "bg-sky-500/10 text-sky-600 dark:text-sky-400")}>{r.type}</span></td>
                  <td className="px-3 py-2 text-muted-foreground">{r.type === "alert" ? `${r.agg}(${r.measure}) on ${r.mart} ${r.op} ${r.threshold}` : (boards.find((b) => b.id === r.board)?.name ?? r.board)}</td>
                  <td className="px-3 py-2"><span className="inline-flex items-center gap-1 text-muted-foreground">{r.channel === "email" ? <Mail className="size-3.5" /> : <Webhook className="size-3.5" />}<span className="max-w-[180px] truncate">{r.target}</span></span></td>
                  <td className="px-3 py-2">
                    <button type="button" role="switch" aria-checked={r.enabled} onClick={() => void toggle(r)} disabled={toggleAction.status === "pending"} className={cn("relative h-5 w-9 rounded-full transition-colors", r.enabled ? "bg-primary" : "bg-muted-foreground/30")}>
                      <span className={cn("absolute top-0.5 size-4 rounded-full bg-white transition-all", r.enabled ? "left-[18px]" : "left-0.5")} />
                    </button>
                  </td>
                  <td className="px-3 py-2 text-right">
                    <div className="flex justify-end gap-1">
                      <Button variant="ghost" size="sm" onClick={() => void run(r.id)} disabled={runAction.status === "pending"} title="Test now"><Send className="size-4" /></Button>
                      <Button variant="ghost" size="sm" onClick={() => void remove(r.id)} disabled={removeAction.status === "pending"} title="Delete"><Trash2 className="size-4 text-destructive" /></Button>
                    </div>
                  </td>
                </tr>
              ))}
              {rules.length === 0 ? <tr><td colSpan={6} className="px-3 py-8 text-center text-muted-foreground">No rules yet. Create an alert or digest.</td></tr> : null}
            </tbody>
          </table>
        </div>
      ) : null}

      <p className="text-xs text-muted-foreground">
        Scheduling: hit <code className="rounded bg-muted px-1 font-mono">POST /api/alerts/run</code> periodically from cron (e.g. every 15 min). Email needs <code className="rounded bg-muted px-1 font-mono">SMTP_HOST/PORT/USER/PASS/FROM</code> env; webhook works out of the box.
      </p>

      {/* Create / edit dialog */}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{edit ? "Edit rule" : "New rule"}</DialogTitle>
            <DialogDescription>Alerts fire when a Gold metric crosses a threshold. Digests summarise a dashboard on a schedule.</DialogDescription>
          </DialogHeader>
          <div className="grid gap-3">
            <div className="grid gap-1.5"><Label>Name</Label><Input value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} placeholder="e.g. Foreign visitors dropped" /></div>
            <div className="grid grid-cols-2 gap-3">
              <div className="grid gap-1.5"><Label>Type</Label>
                <Select value={f.type} onValueChange={(v) => setF({ ...f, type: (v ?? "alert") })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent><SelectItem value="alert">Threshold alert</SelectItem><SelectItem value="digest">Dashboard digest</SelectItem><SelectItem value="freshness">Dataset freshness</SelectItem></SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5"><Label>Delivery</Label>
                <Select value={f.channel} onValueChange={(v) => setF({ ...f, channel: (v ?? "webhook") })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent><SelectItem value="webhook">Webhook (Slack/Discord)</SelectItem><SelectItem value="email">Email (SMTP)</SelectItem></SelectContent>
                </Select>
              </div>
            </div>

            <div className="grid gap-1.5"><Label>Severity</Label>
              {/* The backend's SEVERITIES accepts exactly these five strings
                  and 400s on anything else; "(none)" must send no
                  `severity` field at all, never an empty string or the
                  literal "none" the backend would reject. */}
              <Select
                value={f.severity ?? "none"}
                onValueChange={(v) => setF({ ...f, severity: !v || v === "none" ? undefined : v })}
              >
                <SelectTrigger><SelectValue placeholder="Severity" /></SelectTrigger>
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

            {formFields.freshnessTarget ? (
              <div className="grid gap-1.5">
                <Label>Table (namespace.table)</Label>
                <Input
                  value={f.mart ?? ""}
                  onChange={(e) => setF({ ...f, mart: e.target.value })}
                  placeholder="bronze.orders"
                />
              </div>
            ) : formFields.martMeasure ? (
              <>
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5"><Label>Mart (Gold)</Label>
                    <Select value={f.mart ?? ""} onValueChange={(v) => { const mv = v ?? ""; setF({ ...f, mart: mv, measure: "" }); void loadFields(mv); }}>
                      <SelectTrigger><SelectValue placeholder="pick a mart" /></SelectTrigger>
                      <SelectContent>{marts.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                    </Select>
                    {martsError ? <p className="mt-1 text-xs text-destructive">{martsError}</p> : null}
                  </div>
                  <div className="grid gap-1.5"><Label>Measure</Label>
                    <Select value={f.measure ?? ""} onValueChange={(v) => setF({ ...f, measure: v ?? "" })} disabled={!fields.length}>
                      <SelectTrigger><SelectValue placeholder={fields.length ? "pick" : "pick mart first"} /></SelectTrigger>
                      <SelectContent>{fields.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                    </Select>
                    {fieldsError ? <p className="mt-1 text-xs text-destructive">{fieldsError}</p> : null}
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-3">
                  <div className="grid gap-1.5"><Label>Aggregate</Label>
                    <Select value={f.agg} onValueChange={(v) => setF({ ...f, agg: v ?? "sum" })}>
                      <SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5"><Label>Operator</Label>
                    <Select value={f.op} onValueChange={(v) => setF({ ...f, op: v ?? ">" })}>
                      <SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{OPS.map((o) => <SelectItem key={o} value={o}>{o}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5"><Label>Threshold</Label><Input type="number" value={f.threshold} onChange={(e) => setF({ ...f, threshold: Number(e.target.value) })} /></div>
                </div>
              </>
            ) : (
              <div className="grid gap-1.5"><Label>Dashboard</Label>
                <Select value={f.board ?? ""} onValueChange={(v) => setF({ ...f, board: v ?? "" })}>
                  <SelectTrigger><SelectValue placeholder="pick a dashboard" /></SelectTrigger>
                  <SelectContent>{boards.map((b) => <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>)}</SelectContent>
                </Select>
                {boardsError ? <p className="mt-1 text-xs text-destructive">{boardsError}</p> : null}
              </div>
            )}

            <div className="grid gap-1.5">
              <Label>{f.channel === "email" ? "Recipient email" : "Webhook URL"}</Label>
              <Input value={f.target} onChange={(e) => setF({ ...f, target: e.target.value })} placeholder={f.channel === "email" ? "boss@company.com" : "https://hooks.slack.com/services/…"} />
            </div>

            {formErr ? <p className="text-sm text-destructive">{formErr}</p> : null}
            {saveAction.error ? <p className="text-sm text-destructive">{saveAction.error.message}</p> : null}
          </div>
          <DialogFooter>
            <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
            <Button size="sm" onClick={() => void save()} disabled={busy}>{busy ? "Saving…" : edit ? "Save" : "Create"}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
