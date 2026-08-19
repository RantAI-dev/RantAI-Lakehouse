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

type Rule = {
  id: string; name: string; type: "alert" | "digest";
  mart?: string; measure?: string; agg?: string; op?: string; threshold?: number;
  board?: string; channel: "webhook" | "email"; target: string; enabled: boolean;
};
type RunResult = { id: string; name: string; type: string; fired: boolean; value?: number; delivered?: { ok: boolean; error?: string }; skipped?: string };

const OPS = [">", ">=", "<", "<=", "=="];
const AGGS = ["sum", "avg", "max", "min", "count"];

export function AlertsPage() {
  const [rules, setRules] = React.useState<Rule[]>([]);
  const [marts, setMarts] = React.useState<string[]>([]);
  const [boards, setBoards] = React.useState<{ id: string; name: string }[]>([]);
  const [fields, setFields] = React.useState<string[]>([]);
  const [open, setOpen] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [err, setErr] = React.useState<string | null>(null);
  const [results, setResults] = React.useState<RunResult[] | null>(null);
  const [edit, setEdit] = React.useState<Rule | null>(null);

  // form
  const [f, setF] = React.useState<Rule>({ id: "", name: "", type: "alert", agg: "sum", op: ">", threshold: 0, channel: "webhook", target: "", enabled: true });

  const load = React.useCallback(async () => {
    const [r, b] = await Promise.all([
      fetch("/api/alerts", { cache: "no-store" }).then((x) => x.json()),
      fetch("/api/dashboard/boards", { cache: "no-store" }).then((x) => x.json()),
    ]);
    setRules(r.rules ?? []);
    setBoards((b.boards ?? []).filter((x: { id: string }) => x.id !== "default"));
    fetch("/api/dashboard/fields").then((x) => x.json()).then((j) => setMarts((j.marts ?? []).map((m: { name: string }) => m.name))).catch(() => {});
  }, []);
  React.useEffect(() => { void load(); }, [load]);

  async function loadFields(mart: string) {
    if (!mart) { setFields([]); return; }
    const j = await fetch(`/api/dashboard/fields?mart=${encodeURIComponent(mart)}`).then((x) => x.json());
    setFields(j.measures ?? []);
  }

  function openNew() {
    setEdit(null);
    setF({ id: "", name: "", type: "alert", mart: "", measure: "", agg: "sum", op: ">", threshold: 0, board: boards[0]?.id, channel: "webhook", target: "", enabled: true });
    setFields([]); setErr(null); setOpen(true);
  }
  function openEdit(r: Rule) {
    setEdit(r); setF({ ...r }); setErr(null); setOpen(true);
    if (r.mart) void loadFields(r.mart);
  }

  async function save() {
    setErr(null);
    if (!f.name.trim()) { setErr("Name required."); return; }
    if (!f.target.trim()) { setErr("Webhook URL / email required."); return; }
    setBusy(true);
    try {
      const res = await fetch("/api/alerts", {
        method: edit ? "PUT" : "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify(edit ? { ...f, id: edit.id } : f),
      });
      const j = await res.json();
      if (!res.ok) throw new Error(j?.error ?? "failed");
      setOpen(false); await load();
    } catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }

  async function remove(id: string) {
    await fetch(`/api/alerts?id=${encodeURIComponent(id)}`, { method: "DELETE" });
    await load();
  }
  async function toggle(r: Rule) {
    await fetch("/api/alerts", { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ ...r, enabled: !r.enabled }) });
    await load();
  }
  async function run(only?: string) {
    setBusy(true); setResults(null);
    try {
      const q = only ? `?id=${encodeURIComponent(only)}` : "";
      const j = await fetch(`/api/alerts/run${q}`, { method: "POST" }).then((x) => x.json());
      setResults(j.results ?? []);
    } finally { setBusy(false); }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-xl font-semibold"><Bell className="size-5" /> Alerts &amp; Digests</h1>
          <p className="text-sm text-muted-foreground">Threshold alerts on Gold metrics + scheduled dashboard digests. Delivered via webhook (Slack/Discord) or email.</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => void run()} disabled={busy}><Play className="size-4" /> Run all now</Button>
          <Button size="sm" onClick={openNew}><Plus className="size-4" /> New rule</Button>
        </div>
      </div>

      {results ? (
        <div className="rounded-lg border border-border bg-card/50 p-3 text-sm">
          <p className="mb-2 font-medium">Run results ({results.length})</p>
          <ul className="space-y-1">
            {results.map((r) => (
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
            {results.length === 0 ? <li className="text-muted-foreground">No enabled rules.</li> : null}
          </ul>
        </div>
      ) : null}

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
                  <button type="button" role="switch" aria-checked={r.enabled} onClick={() => void toggle(r)} className={cn("relative h-5 w-9 rounded-full transition-colors", r.enabled ? "bg-primary" : "bg-muted-foreground/30")}>
                    <span className={cn("absolute top-0.5 size-4 rounded-full bg-white transition-all", r.enabled ? "left-[18px]" : "left-0.5")} />
                  </button>
                </td>
                <td className="px-3 py-2 text-right">
                  <div className="flex justify-end gap-1">
                    <Button variant="ghost" size="sm" onClick={() => void run(r.id)} disabled={busy} title="Test now"><Send className="size-4" /></Button>
                    <Button variant="ghost" size="sm" onClick={() => void remove(r.id)} title="Delete"><Trash2 className="size-4 text-destructive" /></Button>
                  </div>
                </td>
              </tr>
            ))}
            {rules.length === 0 ? <tr><td colSpan={6} className="px-3 py-8 text-center text-muted-foreground">No rules yet. Create an alert or digest.</td></tr> : null}
          </tbody>
        </table>
      </div>

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
                <Select value={f.type} onValueChange={(v) => setF({ ...f, type: (v as "alert" | "digest") })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent><SelectItem value="alert">Threshold alert</SelectItem><SelectItem value="digest">Dashboard digest</SelectItem></SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5"><Label>Delivery</Label>
                <Select value={f.channel} onValueChange={(v) => setF({ ...f, channel: (v as "webhook" | "email") })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent><SelectItem value="webhook">Webhook (Slack/Discord)</SelectItem><SelectItem value="email">Email (SMTP)</SelectItem></SelectContent>
                </Select>
              </div>
            </div>

            {f.type === "alert" ? (
              <>
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5"><Label>Mart (Gold)</Label>
                    <Select value={f.mart ?? ""} onValueChange={(v) => { const mv = v ?? ""; setF({ ...f, mart: mv, measure: "" }); void loadFields(mv); }}>
                      <SelectTrigger><SelectValue placeholder="pick a mart" /></SelectTrigger>
                      <SelectContent>{marts.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5"><Label>Measure</Label>
                    <Select value={f.measure ?? ""} onValueChange={(v) => setF({ ...f, measure: v ?? "" })} disabled={!fields.length}>
                      <SelectTrigger><SelectValue placeholder={fields.length ? "pick" : "pick mart first"} /></SelectTrigger>
                      <SelectContent>{fields.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-3">
                  <div className="grid gap-1.5"><Label>Aggregate</Label>
                    <Select value={f.agg ?? "sum"} onValueChange={(v) => setF({ ...f, agg: v ?? "sum" })}>
                      <SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5"><Label>Operator</Label>
                    <Select value={f.op ?? ">"} onValueChange={(v) => setF({ ...f, op: v ?? ">" })}>
                      <SelectTrigger><SelectValue /></SelectTrigger><SelectContent>{OPS.map((o) => <SelectItem key={o} value={o}>{o}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5"><Label>Threshold</Label><Input type="number" value={f.threshold ?? 0} onChange={(e) => setF({ ...f, threshold: Number(e.target.value) })} /></div>
                </div>
              </>
            ) : (
              <div className="grid gap-1.5"><Label>Dashboard</Label>
                <Select value={f.board ?? ""} onValueChange={(v) => setF({ ...f, board: v ?? "" })}>
                  <SelectTrigger><SelectValue placeholder="pick a dashboard" /></SelectTrigger>
                  <SelectContent>{boards.map((b) => <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>)}</SelectContent>
                </Select>
              </div>
            )}

            <div className="grid gap-1.5">
              <Label>{f.channel === "email" ? "Recipient email" : "Webhook URL"}</Label>
              <Input value={f.target} onChange={(e) => setF({ ...f, target: e.target.value })} placeholder={f.channel === "email" ? "boss@company.com" : "https://hooks.slack.com/services/…"} />
            </div>

            {err ? <p className="text-sm text-destructive">{err}</p> : null}
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
