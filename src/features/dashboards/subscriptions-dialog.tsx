"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useService } from "@/hooks/use-service";
import { normalizeFilters } from "@/lib/dashboard-filter-state";
import { reportService } from "@/services";
import type {
  ReportPerson,
  ReportSchedule,
  ReportSubscription,
  ReportSubscriptionInput,
} from "@/services/contracts/reports";
import type { FilterDef } from "@/services/clients/bi-store";
import type { ReadState } from "./chart-download";
import { clock, describeAttachments, describeSchedule, describeStatus, WEEKDAYS } from "./report-words";

/**
 * What a report is made as (feature decision 1). Shown above the recipient list
 * so nobody adds a person without knowing they will see what the creator sees.
 */
export const AS_CREATOR_SENTENCE =
  "This report is made as you: every recipient, inside or outside the company, sees the data you are allowed to see, with your masking and row filters.";

/** Shown while the deployment has no `SMTP_HOST` (feature decision 9). */
export const EMAIL_NOT_SET_UP = "Email is not set up on this deployment.";

const selectClass = "h-8 rounded-md border border-input bg-background px-2 text-sm";

type DashboardChart = { readonly id: string; readonly title: string };

/** The dashboard's ⋯ > "Subscriptions…": the list, and the form to create or change one. */
export function SubscriptionsDialog({
  open, onOpenChange, board, dashName, charts, readState, reportZone,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly board: string;
  readonly dashName: string;
  /** The data charts of this dashboard a report may attach (static tiles left out). */
  readonly charts: readonly DashboardChart[];
  /** What the page is showing now: a new subscription starts with these filters, parameters and grain. */
  readonly readState: ReadState;
  /** The report time zone, the default of a new subscription. */
  readonly reportZone: string;
}) {
  const [version, setVersion] = React.useState(0);
  const list = useService((signal) => reportService.list(board, signal), [board, version]);
  const [editing, setEditing] = React.useState<ReportSubscription | "new" | null>(null);
  const [history, setHistory] = React.useState<ReportSubscription | null>(null);
  const reload = () => setVersion((v) => v + 1);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader><DialogTitle>Subscriptions · {dashName}</DialogTitle></DialogHeader>
        {editing ? (
          <SubscriptionForm
            board={board}
            existing={editing === "new" ? null : editing}
            charts={charts}
            readState={readState}
            reportZone={reportZone}
            pdfSupported={list.status === "success" ? list.data.pdfSupported : true}
            onCancel={() => setEditing(null)}
            onSaved={() => { setEditing(null); reload(); }}
          />
        ) : history ? (
          <History subscription={history} onBack={() => setHistory(null)} />
        ) : (
          <>
            {list.status === "loading" ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
            {list.status === "error" ? <p role="alert" className="text-sm text-destructive">{list.error.message}</p> : null}
            {list.status === "success" ? (
              <>
                {!list.data.emailSetUp ? (
                  <p role="status" className="rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
                    {EMAIL_NOT_SET_UP} A subscription can be saved, but nothing is sent until it is.
                  </p>
                ) : null}
                {list.data.items.length === 0 ? <p className="text-sm text-muted-foreground">No subscriptions on this dashboard yet.</p> : null}
                <ul className="flex flex-col gap-2" aria-label="Subscriptions">
                  {list.data.items.map((s) => (
                    <SubscriptionRow key={s.id} s={s} onEdit={() => setEditing(s)} onHistory={() => setHistory(s)} onChanged={reload} />
                  ))}
                </ul>
              </>
            ) : null}
            <DialogFooter>
              <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>Close</Button>
              <Button size="sm" onClick={() => setEditing("new")}>New subscription</Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function SubscriptionRow({
  s, onEdit, onHistory, onChanged,
}: {
  readonly s: ReportSubscription;
  readonly onEdit: () => void;
  readonly onHistory: () => void;
  readonly onChanged: () => void;
}) {
  const [busy, setBusy] = React.useState(false);
  const [note, setNote] = React.useState<string | null>(null);
  async function run(fn: () => Promise<string | void>) {
    setBusy(true); setNote(null);
    try {
      const done = await fn();
      if (done) setNote(done);
      onChanged();
    } catch (e) {
      setNote(e instanceof Error ? e.message : "That did not work.");
    } finally {
      setBusy(false);
    }
  }
  const people = s.recipients.map((r) => r.name ?? r.email ?? "").filter(Boolean);
  return (
    <li className="flex flex-col gap-1 rounded-lg border p-3 text-sm">
      <div className="font-medium">{describeSchedule(s.schedule, s.timeZone)}</div>
      <div className="text-xs text-muted-foreground">
        {describeAttachments(s.attachments)} · to {people.length > 3 ? `${people.slice(0, 3).join(", ")} and ${people.length - 3} more` : people.join(", ")}
        {s.skipWhenEmpty ? " · skipped when every chart is empty" : ""}
      </div>
      <div className="text-xs">{describeStatus(s)}</div>
      {note ? <p role="status" className="text-xs">{note}</p> : null}
      {s.canEdit !== false ? (
        <div className="flex flex-wrap gap-1.5 pt-1">
          <Button variant="outline" size="xs" disabled={busy} onClick={() => void run(async () => {
            const d = await reportService.sendNow(s.id);
            return d.status === "sent" ? "Sent. Check the inbox." : d.status === "skipped" ? `Skipped. ${d.reason ?? ""}` : `Not sent. ${d.reason ?? ""}`;
          })}>Send now</Button>
          <Button variant="outline" size="xs" disabled={busy} onClick={() => void run(async () => {
            // RPT-1 review fix (SHOULD-FIX) R4: pause and resume send only the flag.
            await reportService.setPaused(s.id, !s.paused);
          })}>{s.paused ? "Resume" : "Pause"}</Button>
          <Button variant="outline" size="xs" disabled={busy} onClick={onEdit}>Edit</Button>
          <Button variant="outline" size="xs" disabled={busy} onClick={onHistory}>History</Button>
          <Button variant="ghost" size="xs" disabled={busy} onClick={() => void run(() => reportService.remove(s.id))}>Delete</Button>
        </div>
      ) : null}
    </li>
  );
}

function History({ subscription, onBack }: { readonly subscription: ReportSubscription; readonly onBack: () => void }) {
  const state = useService((signal) => reportService.deliveries(subscription.id, signal), [subscription.id]);
  return (
    <>
      <p className="text-sm font-medium">{describeSchedule(subscription.schedule, subscription.timeZone)}</p>
      {state.status === "loading" ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
      {state.status === "error" ? <p role="alert" className="text-sm text-destructive">{state.error.message}</p> : null}
      {state.status === "success" && state.data.length === 0 ? <p className="text-sm text-muted-foreground">Nothing has been sent yet. Records are kept 90 days.</p> : null}
      {state.status === "success" && state.data.length > 0 ? (
        <ul className="flex flex-col gap-1.5" aria-label="Deliveries">
          {state.data.map((d) => (
            <li key={d.id} className="rounded-md border px-3 py-2 text-xs">
              <span className="font-medium capitalize">{d.status}</span>
              {d.origin === "manual" ? " (send now)" : ""} · {new Date(d.startedAt.replace(" ", "T")).toLocaleString() === "Invalid Date" ? d.startedAt : new Date(d.startedAt.replace(" ", "T")).toLocaleString()} · {d.recipientCount} {d.recipientCount === 1 ? "recipient" : "recipients"}
              {d.reason ? <div className="text-muted-foreground">{d.reason}</div> : null}
            </li>
          ))}
        </ul>
      ) : null}
      <DialogFooter><Button variant="ghost" size="sm" onClick={onBack}>Back</Button></DialogFooter>
    </>
  );
}

type Kind = ReportSchedule["kind"];

function SubscriptionForm({
  board, existing, charts, readState, reportZone, pdfSupported, onCancel, onSaved,
}: {
  readonly board: string;
  readonly existing: ReportSubscription | null;
  readonly charts: readonly DashboardChart[];
  readonly readState: ReadState;
  readonly reportZone: string;
  readonly pdfSupported: boolean;
  readonly onCancel: () => void;
  readonly onSaved: () => void;
}) {
  const sched = existing?.schedule ?? null;
  const [kind, setKind] = React.useState<Kind>(sched?.kind ?? "weekly");
  const [minute, setMinute] = React.useState(sched?.minute ?? 0);
  const [time, setTime] = React.useState(sched && "hour" in sched ? clock(sched.hour, sched.minute) : "08:00");
  const [weekday, setWeekday] = React.useState(sched?.kind === "weekly" ? sched.weekday : 1);
  const [monthDay, setMonthDay] = React.useState<number | "last">(sched?.kind === "monthly" ? sched.monthDay : 1);
  const [zone, setZone] = React.useState(existing?.timeZone ?? reportZone);
  const [pdf, setPdf] = React.useState(existing ? Boolean(existing.attachments.pdf) : pdfSupported);
  const [paper, setPaper] = React.useState(existing?.attachments.pdf?.paper ?? "a4");
  const [orientation, setOrientation] = React.useState(existing?.attachments.pdf?.orientation ?? "portrait");
  const [files, setFiles] = React.useState<Record<string, "none" | "xlsx" | "csv">>(() =>
    Object.fromEntries((existing?.attachments.charts ?? []).map((c) => [c.chart, c.format === "csv" ? "csv" : "xlsx"] as const)));
  const [values, setValues] = React.useState(existing?.attachments.charts[0]?.values ?? "formatted");
  const [skip, setSkip] = React.useState(existing?.skipWhenEmpty ?? false);
  const [people, setPeople] = React.useState<{ userId: string; label: string }[]>(
    () => (existing?.recipients ?? []).filter((r) => r.userId).map((r) => ({ userId: r.userId as string, label: r.name ?? r.email ?? "" })));
  const [addresses, setAddresses] = React.useState<string[]>(() => (existing?.recipients ?? []).filter((r) => !r.userId && r.email).map((r) => r.email as string));
  const [query, setQuery] = React.useState("");
  const [found, setFound] = React.useState<ReportPerson[]>([]);
  const [address, setAddress] = React.useState("");
  const current = React.useMemo(() => normalizeFilters([...readState.filters]) as FilterDef[], [readState.filters]);
  const [filters, setFilters] = React.useState<unknown[] | null>(existing ? existing.filters : current);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!query.trim()) { setFound([]); return; }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      reportService.people(query.trim(), controller.signal).then(setFound).catch(() => setFound([]));
    }, 200);
    return () => { clearTimeout(timer); controller.abort(); };
  }, [query]);

  const [hh, mm] = time.split(":").map((n) => Number.parseInt(n, 10));
  const schedule: ReportSchedule | null = (() => {
    if (kind === "hourly") return { kind, minute };
    if (!Number.isFinite(hh) || !Number.isFinite(mm)) return null;
    if (kind === "daily") return { kind, hour: hh, minute: mm };
    if (kind === "weekly") return { kind, weekday, hour: hh, minute: mm };
    return { kind, monthDay, hour: hh, minute: mm };
  })();

  function addAddress() {
    const a = address.trim().toLowerCase();
    if (!/^[^\s@,;<>]+@[^\s@,;<>]+\.[^\s@,;<>]+$/.test(a) || addresses.includes(a)) return;
    setAddresses([...addresses, a]);
    setAddress("");
  }

  async function save() {
    if (!schedule) { setError("Choose a time."); return; }
    const chosen = Object.entries(files).filter(([, f]) => f !== "none").map(([chart, format]) => ({ chart, format, values }));
    const input: ReportSubscriptionInput = {
      schedule,
      timeZone: zone.trim() || reportZone,
      attachments: { pdf: pdf ? { paper, orientation } : null, charts: chosen },
      filters: filters && filters.length ? filters : null,
      params: existing ? existing.params : parseJson(readState.params),
      grain: existing ? existing.grain : (readState.grain ?? null),
      skipWhenEmpty: skip,
      recipients: [...people.map((p) => ({ userId: p.userId })), ...addresses.map((email) => ({ email }))],
    };
    setBusy(true); setError(null);
    try {
      if (existing) await reportService.update(existing.id, input);
      else await reportService.create({ ...input, board });
      onSaved();
    } catch (e) {
      // The route's own sentence: an outside domain not listed, 51 recipients, a user who is not active.
      setError(e instanceof Error ? e.message : "The subscription could not be saved.");
    } finally {
      setBusy(false);
    }
  }

  const summary = schedule ? describeSchedule(schedule, zone.trim() || reportZone) : "";
  return (
    <div className="flex flex-col gap-4">
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 text-sm font-medium">When</legend>
        <div className="flex flex-wrap items-end gap-3">
          <Label className="flex flex-col gap-1 text-xs font-normal">Repeat
            <select aria-label="Repeat" className={selectClass} value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
              <option value="hourly">Every hour</option>
              <option value="daily">Every day</option>
              <option value="weekly">Every week</option>
              <option value="monthly">Every month</option>
            </select>
          </Label>
          {kind === "weekly" ? (
            <Label className="flex flex-col gap-1 text-xs font-normal">On
              <select aria-label="Weekday" className={selectClass} value={weekday} onChange={(e) => setWeekday(Number(e.target.value))}>
                {WEEKDAYS.map((d, i) => <option key={d} value={i + 1}>{d}</option>)}
              </select>
            </Label>
          ) : null}
          {kind === "monthly" ? (
            <Label className="flex flex-col gap-1 text-xs font-normal">On day
              <select aria-label="Day of the month" className={selectClass} value={String(monthDay)} onChange={(e) => setMonthDay(e.target.value === "last" ? "last" : Number(e.target.value))}>
                {Array.from({ length: 28 }, (_, i) => <option key={i + 1} value={String(i + 1)}>{i + 1}</option>)}
                <option value="last">The last day</option>
              </select>
            </Label>
          ) : null}
          {kind === "hourly" ? (
            <Label className="flex flex-col gap-1 text-xs font-normal">Minutes past the hour
              <Input aria-label="Minute" type="number" min={0} max={59} value={minute} onChange={(e) => setMinute(Math.min(59, Math.max(0, Number(e.target.value) || 0)))} className="w-24" />
            </Label>
          ) : (
            <Label className="flex flex-col gap-1 text-xs font-normal">At
              <Input aria-label="Time" type="time" value={time} onChange={(e) => setTime(e.target.value)} className="w-28" />
            </Label>
          )}
          <Label className="flex flex-col gap-1 text-xs font-normal">Time zone
            <Input aria-label="Time zone" value={zone} onChange={(e) => setZone(e.target.value)} className="w-44" />
          </Label>
        </div>
        <p className="text-xs text-muted-foreground" aria-live="polite">{summary}</p>
      </fieldset>

      <fieldset className="flex flex-col gap-1.5">
        <legend className="mb-1 text-sm font-medium">What to send</legend>
        <Label className="flex items-center gap-2 font-normal">
          <input type="checkbox" checked={pdf} disabled={!pdfSupported} onChange={(e) => setPdf(e.target.checked)} />
          The dashboard as a PDF
        </Label>
        {!pdfSupported ? <p className="text-xs text-muted-foreground">The PDF service is not set up on this deployment, so a PDF cannot be attached.</p> : null}
        {pdf ? (
          <div className="ml-6 flex gap-3">
            <select aria-label="Paper" className={selectClass} value={paper} onChange={(e) => setPaper(e.target.value)}>
              <option value="a4">A4</option><option value="letter">Letter</option>
            </select>
            <select aria-label="Orientation" className={selectClass} value={orientation} onChange={(e) => setOrientation(e.target.value)}>
              <option value="portrait">Portrait</option><option value="landscape">Landscape</option>
            </select>
          </div>
        ) : null}
        {charts.length ? <p className="pt-1 text-xs text-muted-foreground">Chart data (up to 10 files, 20 MB in all; a file that does not fit is left out and the email says so)</p> : null}
        {charts.map((c) => (
          <div key={c.id} className="flex items-center gap-2 text-sm">
            <span className="min-w-0 flex-1 truncate">{c.title}</span>
            <select aria-label={`File for ${c.title}`} className={selectClass} value={files[c.id] ?? "none"} onChange={(e) => setFiles({ ...files, [c.id]: e.target.value as "none" | "xlsx" | "csv" })}>
              <option value="none">Not attached</option><option value="xlsx">Excel</option><option value="csv">CSV</option>
            </select>
          </div>
        ))}
        {Object.values(files).some((f) => f !== "none") ? (
          <select aria-label="Values" className={`${selectClass} w-fit`} value={values} onChange={(e) => setValues(e.target.value)}>
            <option value="formatted">Values as on the chart</option><option value="raw">Raw values</option>
          </select>
        ) : null}
      </fieldset>

      <fieldset className="flex flex-col gap-1.5">
        <legend className="mb-1 text-sm font-medium">Filters</legend>
        <p className="text-xs text-muted-foreground">
          {filters && filters.length ? `${filters.length} ${filters.length === 1 ? "filter" : "filters"} will be applied to this report.` : "No filters: the dashboard's own."}
          {" "}
          <button type="button" className="underline" onClick={() => setFilters(current)}>Use the filters on screen</button>
          {" · "}
          <button type="button" className="underline" onClick={() => setFilters(null)}>Clear</button>
        </p>
        <Label className="flex items-center gap-2 font-normal">
          <input type="checkbox" checked={skip} onChange={(e) => setSkip(e.target.checked)} />
          Skip when empty: send nothing when every chart has no rows
        </Label>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 text-sm font-medium">Recipients</legend>
        <p className="text-xs text-muted-foreground">{AS_CREATOR_SENTENCE}</p>
        <ul className="flex flex-wrap gap-1.5" aria-label="Chosen recipients">
          {people.map((p) => (
            <li key={p.userId} className="flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs">{p.label}
              <button type="button" aria-label={`Remove ${p.label}`} onClick={() => setPeople(people.filter((x) => x.userId !== p.userId))}>×</button></li>
          ))}
          {addresses.map((a) => (
            <li key={a} className="flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs">{a}
              <button type="button" aria-label={`Remove ${a}`} onClick={() => setAddresses(addresses.filter((x) => x !== a))}>×</button></li>
          ))}
        </ul>
        <Input aria-label="Find a person" placeholder="Find a person by name or email" value={query} onChange={(e) => setQuery(e.target.value)} autoComplete="off" />
        {found.length ? (
          <ul className="flex flex-col gap-1" aria-label="People found">
            {found.filter((f) => !people.some((p) => p.userId === f.id)).map((f) => (
              <li key={f.id}><button type="button" className="text-left text-sm underline" onClick={() => { setPeople([...people, { userId: f.id, label: f.name }]); setQuery(""); }}>{f.name} <span className="text-muted-foreground">{f.email}</span></button></li>
            ))}
          </ul>
        ) : null}
        <div className="flex gap-2">
          <Input aria-label="Outside address" placeholder="Outside email address (its domain must be allowed)" value={address} onChange={(e) => setAddress(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addAddress(); } }} autoComplete="off" />
          <Button variant="outline" size="sm" onClick={addAddress}>Add address</Button>
        </div>
      </fieldset>

      {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
      <DialogFooter>
        <Button variant="ghost" size="sm" disabled={busy} onClick={onCancel}>Cancel</Button>
        <Button size="sm" disabled={busy} onClick={() => void save()}>{busy ? "Saving…" : existing ? "Save changes" : "Subscribe"}</Button>
      </DialogFooter>
    </div>
  );
}

/** `paramsQuery(...)` text back to the object the API stores; none when absent or not an object. */
function parseJson(text: string | null | undefined): Record<string, unknown> | null {
  if (!text) return null;
  try {
    const v: unknown = JSON.parse(text);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}
