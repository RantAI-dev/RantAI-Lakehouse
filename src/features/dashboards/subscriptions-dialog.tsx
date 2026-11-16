"use client";

import * as React from "react";
import { ArrowLeft, CalendarClock, FileSpreadsheet, Loader2, X } from "lucide-react";
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { FormField, FormGroup, FormHint, SwitchRow } from "@/components/patterns/form-fields";
import { HelpHint } from "@/components/patterns/help-hint";
import { SelectField } from "@/components/patterns/select-field";
import { Button } from "@/components/ui/button";
import {
  Combobox, ComboboxContent, ComboboxEmpty, ComboboxInput, ComboboxItem, ComboboxList,
} from "@/components/ui/combobox";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useService, useServiceAction } from "@/hooks/use-service";
import { normalizeFilters } from "@/lib/dashboard-filter-state";
import { cn } from "@/lib/utils";
import { reportService } from "@/services";
import type {
  ReportSchedule,
  ReportSubscription,
  ReportSubscriptionInput,
} from "@/services/contracts/reports";
import type { FilterDef } from "@/services/clients/bi-store";
import type { ReadState } from "./chart-download";
import { clock, describeAttachments, describeNotSent, describeSchedule, describeStatus, WEEKDAYS } from "./report-words";
import { ReportRecipientsField, type ChosenRecipient } from "./report-recipients-field";
import { TimeZoneSelect } from "./time-zone-select";

/**
 * What a report is made as (feature decision 1). Shown above the recipient list
 * so nobody adds a person without knowing they will see what the creator sees.
 */
export const AS_CREATOR_SENTENCE =
  "This report is made as you: every recipient, inside or outside the company, sees the data you are allowed to see, with your masking and row filters.";

/** Next to "Skip when empty" on a dashboard that has only single-value charts (RPT-1 QA fix Q3). */
export const ALWAYS_SENT_SENTENCE =
  "This dashboard has no chart that groups or lists rows, so it is always sent: a single-value chart (a KPI, a progress bar, a gauge) always has a value.";

/** Shown while the deployment has no `SMTP_HOST` (feature decision 9). */
export const EMAIL_NOT_SET_UP = "Email is not set up on this deployment.";

/** `WEB-9` D3: the schedule choices, in the order the form always offered them. */
const REPEAT_OPTIONS = [
  { value: "hourly", label: "Every hour" },
  { value: "daily", label: "Every day" },
  { value: "weekly", label: "Every week" },
  { value: "monthly", label: "Every month" },
];

const WEEKDAY_OPTIONS = WEEKDAYS.map((d, i) => ({ value: String(i + 1), label: d }));

const MONTH_DAY_OPTIONS = [
  ...Array.from({ length: 28 }, (_, i) => ({ value: String(i + 1), label: String(i + 1) })),
  { value: "last", label: "The last day" },
];

const PAPER_OPTIONS = [
  { value: "a4", label: "A4" },
  { value: "letter", label: "Letter" },
];

const ORIENTATION_OPTIONS = [
  { value: "portrait", label: "Portrait" },
  { value: "landscape", label: "Landscape" },
];

const FILE_FORMATS = [
  { value: "xlsx", label: "Excel" },
  { value: "csv", label: "CSV" },
] as const;

type FileFormat = (typeof FILE_FORMATS)[number]["value"];

const VALUES_OPTIONS = [
  { value: "formatted", label: "Values as on the chart" },
  { value: "raw", label: "Raw values" },
];

/** `grouped`: the chart groups or lists rows, so "Skip when empty" can judge it (RPT-1 QA fix Q3). */
type DashboardChart = { readonly id: string; readonly title: string; readonly grouped: boolean };

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
  // Owner QA 2026-11-16 (one action, one button): the form has no Cancel; the
  // back arrow, Esc and the dialog's X leave it, and leaving with changes not
  // saved asks first (DESIGN.md §3 "Buang perubahan?").
  const [dirty, setDirty] = React.useState(false);
  const [leaving, setLeaving] = React.useState<(() => void) | null>(null);
  function leave(then: () => void) {
    if (editing && dirty) setLeaving(() => then);
    else then();
  }
  function backToList() {
    setEditing(null);
    setHistory(null);
    setDirty(false);
  }
  const title = editing === "new" ? "New subscription" : editing ? "Edit subscription" : history ? "Send history" : `Subscriptions · ${dashName}`;

  return (
    <>
    <Dialog open={open} onOpenChange={(next) => { if (next) onOpenChange(true); else leave(() => { backToList(); onOpenChange(false); }); }}>
      <DialogContent className="max-h-[85vh] gap-5 overflow-y-auto sm:max-w-xl">
        <DialogHeader className="flex-row items-center gap-1.5">
          {editing || history ? (
            <Button variant="ghost" size="icon-sm" className="-ml-1.5 shrink-0" aria-label="Back to the subscriptions" onClick={() => leave(backToList)}>
              <ArrowLeft aria-hidden />
            </Button>
          ) : null}
          <DialogTitle className="min-w-0 truncate leading-snug" title={title}>{title}</DialogTitle>
        </DialogHeader>
        {editing ? (
          <SubscriptionForm
            board={board}
            existing={editing === "new" ? null : editing}
            charts={charts}
            readState={readState}
            reportZone={reportZone}
            pdfSupported={list.status === "success" ? list.data.pdfSupported : true}
            onDirtyChange={setDirty}
            onSaved={() => { backToList(); reload(); }}
          />
        ) : history ? (
          <History subscription={history} />
        ) : (
          <>
            {list.status === "loading" ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
            {list.status === "error" ? <p role="alert" className="text-sm text-destructive">{list.error.message}</p> : null}
            {list.status === "success" ? (
              <>
                {!list.data.emailSetUp ? (
                  <p role="status" className="rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
                    {EMAIL_NOT_SET_UP} A subscription can be saved, but nothing is sent until it is.
                  </p>
                ) : null}
                {list.data.items.length === 0 ? <p className="text-sm text-muted-foreground">No subscriptions on this dashboard yet.</p> : null}
                {list.data.items.length > 0 ? (
                  <ul className="flex min-w-0 flex-col gap-2" aria-label="Subscriptions">
                    {list.data.items.map((s) => (
                      <SubscriptionRow key={s.id} s={s} dashName={dashName} onEdit={() => setEditing(s)} onHistory={() => setHistory(s)} onChanged={reload} />
                    ))}
                  </ul>
                ) : null}
              </>
            ) : null}
            <DialogFooter>
              <Button size="sm" onClick={() => setEditing("new")}>New subscription</Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
    <ConfirmActionDialog
      open={leaving !== null}
      onOpenChange={(o) => { if (!o) setLeaving(null); }}
      title="Discard your changes?"
      description="The subscription keeps what it had before; what you changed here is not saved."
      confirmLabel="Discard changes"
      destructive
      onConfirm={() => { const then = leaving; setLeaving(null); then?.(); }}
    />
    </>
  );
}

function SubscriptionRow({
  s, dashName, onEdit, onHistory, onChanged,
}: {
  readonly s: ReportSubscription;
  readonly dashName: string;
  readonly onEdit: () => void;
  readonly onHistory: () => void;
  readonly onChanged: () => void;
}) {
  // BI-35: the three row actions go through `useServiceAction`, like the rest of
  // the console, so a request is aborted when the dialog closes and an aborted
  // call never shows as an error.
  const sendAction = useServiceAction((signal, id: string) => reportService.sendNow(id, signal));
  const pauseAction = useServiceAction((signal, id: string, paused: boolean) => reportService.setPaused(id, paused, signal));
  const removeAction = useServiceAction((signal, id: string) => reportService.remove(id, signal));
  const busy = [sendAction, pauseAction, removeAction].some((a) => a.status === "pending");
  // RPT-1 QA fix Q1: a send can take a minute with a PDF, so this row says it is
  // sending (not only greyed) until the answer arrives; other actions stay quiet.
  const sending = sendAction.status === "pending";
  const [note, setNote] = React.useState<string | null>(null);
  // BI-35: Delete asks first, naming the subscription.
  const [confirmingDelete, setConfirmingDelete] = React.useState(false);
  const failure = confirmingDelete ? null : (sendAction.error ?? pauseAction.error ?? removeAction.error);
  function begin() {
    setNote(null);
    sendAction.reset(); pauseAction.reset(); removeAction.reset();
  }
  async function send() {
    begin();
    const d = await sendAction.run(s.id);
    if (d) {
      setNote(d.status === "sent" ? "Sent. Check the inbox." : d.status === "skipped" ? `Skipped. ${d.reason ?? ""}` : `Not sent. ${d.reason ?? ""}`);
      // A failed delivery is recorded too, so the list is read again.
      onChanged();
    }
  }
  async function pause() {
    begin();
    // RPT-1 review fix (SHOULD-FIX) R4: pause and resume send only the flag.
    if (await pauseAction.run(s.id, !s.paused)) onChanged();
  }
  async function remove() {
    begin();
    // The confirmation stays open on a failure and shows the reason.
    if ((await removeAction.run(s.id)) !== null) onChanged();
  }
  const people = s.recipients.map((r) => r.name ?? r.email ?? "").filter(Boolean);
  // QA LH-RPT-1-06 (RPT-2, DEC-18): while e-mail is not set up, a failure for
  // that reason is not counted and does not pause anything, so the status line
  // would read "Failed. …" and say nothing about it being the installation's
  // state. The "Not sent" line stands in its place; a paused subscription keeps
  // its own line under it.
  const notSent = describeNotSent(s);
  return (
    <li className="flex min-w-0 flex-col gap-1 rounded-lg bg-muted/40 p-3 text-sm">
      <div className="font-medium">{describeSchedule(s.schedule, s.timeZone)}</div>
      {/* Owner QA 2026-11-16: one line; the whole of it in the title. */}
      {(() => {
        const meta = `${describeAttachments(s.attachments)} · to ${people.length > 3 ? `${people.slice(0, 3).join(", ")} and ${people.length - 3} more` : people.join(", ")}${s.skipWhenEmpty ? " · skipped when every chart is empty" : ""}`;
        return <div className="truncate text-xs text-muted-foreground" title={meta}>{meta}</div>;
      })()}
      {notSent !== null ? (
        <p className="text-xs font-medium text-amber-700 dark:text-amber-400">{notSent}</p>
      ) : null}
      {notSent === null || s.paused ? <div className="text-xs">{describeStatus(s)}</div> : null}
      {sending ? (
        <p role="status" className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />Sending…
        </p>
      ) : failure ? <p role="alert" className="text-xs text-destructive">{failure.message}</p>
        : note ? <p role="status" className="text-xs">{note}</p> : null}
      {s.canEdit !== false ? (
        <div className="-ml-2 flex flex-wrap gap-0.5 pt-1">
          <Button variant="ghost" size="xs" disabled={busy} onClick={() => void send()}>{sending ? "Sending…" : "Send now"}</Button>
          <Button variant="ghost" size="xs" disabled={busy} onClick={() => void pause()}>{s.paused ? "Resume" : "Pause"}</Button>
          <Button variant="ghost" size="xs" disabled={busy} onClick={onEdit}>Edit</Button>
          <Button variant="ghost" size="xs" disabled={busy} onClick={onHistory}>History</Button>
          <Button variant="ghost" size="xs" disabled={busy} className="text-muted-foreground hover:text-destructive" onClick={() => { begin(); setConfirmingDelete(true); }}>Delete</Button>
        </div>
      ) : null}
      {confirmingDelete ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => { if (!open && removeAction.status !== "pending") setConfirmingDelete(false); }}
          title="Delete this subscription?"
          description={`${describeSchedule(s.schedule, s.timeZone)} on ${dashName} will stop for everyone it goes to. Its send history is deleted too.`}
          confirmLabel="Delete subscription"
          confirming={removeAction.status === "pending"}
          destructive
          onConfirm={() => void remove()}
        >
          {removeAction.error ? <p role="alert" className="text-sm text-destructive">{removeAction.error.message}</p> : null}
        </ConfirmActionDialog>
      ) : null}
    </li>
  );
}

function History({ subscription }: { readonly subscription: ReportSubscription }) {
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
            <li key={d.id} className="rounded-lg bg-muted/40 px-3 py-2 text-xs">
              <span className="font-medium capitalize">{d.status}</span>
              {d.origin === "manual" ? " (send now)" : ""} · {new Date(d.startedAt.replace(" ", "T")).toLocaleString() === "Invalid Date" ? d.startedAt : new Date(d.startedAt.replace(" ", "T")).toLocaleString()} · {d.recipientCount} {d.recipientCount === 1 ? "recipient" : "recipients"}
              {d.reason ? <div className="text-muted-foreground">{d.reason}</div> : null}
            </li>
          ))}
        </ul>
      ) : null}
    </>
  );
}

type Kind = ReportSchedule["kind"];

function SubscriptionForm({
  board, existing, charts, readState, reportZone, pdfSupported, onDirtyChange, onSaved,
}: {
  readonly board: string;
  readonly existing: ReportSubscription | null;
  readonly charts: readonly DashboardChart[];
  readonly readState: ReadState;
  readonly reportZone: string;
  readonly pdfSupported: boolean;
  /** Whether the form holds changes not saved, so leaving it can ask first. */
  readonly onDirtyChange: (dirty: boolean) => void;
  readonly onSaved: () => void;
}) {
  const sched = existing?.schedule ?? null;
  const [kind, setKind] = React.useState<Kind>(sched?.kind ?? "weekly");
  const [minute, setMinute] = React.useState(sched?.minute ?? 0);
  const [time, setTime] = React.useState(sched && "hour" in sched ? clock(sched.hour, sched.minute) : "08:00");
  const [weekday, setWeekday] = React.useState(sched?.kind === "weekly" ? sched.weekday : 1);
  const [monthDay, setMonthDay] = React.useState<number | "last">(sched?.kind === "monthly" ? sched.monthDay : 1);
  // A new subscription starts in the deployment's report time zone (the
  // `reportZone` prop, Admin > Settings); the picker offers the browser's own
  // zone first. An existing one keeps the zone it was saved with.
  const [zone, setZone] = React.useState(existing?.timeZone ?? reportZone);
  const [pdf, setPdf] = React.useState(existing ? Boolean(existing.attachments.pdf) : pdfSupported);
  const [paper, setPaper] = React.useState(existing?.attachments.pdf?.paper ?? "a4");
  const [orientation, setOrientation] = React.useState(existing?.attachments.pdf?.orientation ?? "portrait");
  // The attached chart files, in the order chosen; a chart not attached has no entry.
  const [files, setFiles] = React.useState<Record<string, FileFormat>>(() =>
    Object.fromEntries((existing?.attachments.charts ?? []).map((c) => [c.chart, c.format === "csv" ? "csv" : "xlsx"] as const)));
  const [values, setValues] = React.useState(existing?.attachments.charts[0]?.values ?? "formatted");
  const [skip, setSkip] = React.useState(existing?.skipWhenEmpty ?? false);
  const [recipients, setRecipients] = React.useState<ChosenRecipient[]>(() =>
    (existing?.recipients ?? []).flatMap((r): ChosenRecipient[] =>
      r.userId ? [{ key: `u:${r.userId}`, userId: r.userId, label: r.name ?? r.email ?? "", detail: r.email ?? null }]
      : r.email ? [{ key: `e:${r.email}`, email: r.email, label: r.email }]
      : []));
  const current = React.useMemo(() => normalizeFilters([...readState.filters]) as FilterDef[], [readState.filters]);
  const [filters, setFilters] = React.useState<unknown[] | null>(existing ? existing.filters : current);
  // BI-35: the save goes through `useServiceAction`; `error` holds only the
  // form's own check ("Choose a time."), the route's sentence is the action's.
  const saveAction = useServiceAction((signal, id: string | null, input: ReportSubscriptionInput) =>
    id ? reportService.update(id, input, signal) : reportService.create({ ...input, board }, signal));
  const busy = saveAction.status === "pending";
  const [error, setError] = React.useState<string | null>(null);
  const shownError = error ?? saveAction.error?.message ?? null;
  const ids = React.useId();

  const [hh, mm] = time.split(":").map((n) => Number.parseInt(n, 10));
  const schedule: ReportSchedule | null = (() => {
    if (kind === "hourly") return { kind, minute };
    if (!Number.isFinite(hh) || !Number.isFinite(mm)) return null;
    if (kind === "daily") return { kind, hour: hh, minute: mm };
    if (kind === "weekly") return { kind, weekday, hour: hh, minute: mm };
    return { kind, monthDay, hour: hh, minute: mm };
  })();
  const effectiveZone = zone.trim() || reportZone;

  // What leaving would lose: the form's choices, compared with how it opened.
  const snapshot = JSON.stringify([kind, minute, time, weekday, monthDay, effectiveZone, pdf, paper, orientation, files, values, skip, recipients.map((r) => r.key), filters]);
  const opened = React.useRef(snapshot);
  React.useEffect(() => { onDirtyChange(snapshot !== opened.current); }, [snapshot, onDirtyChange]);

  async function save() {
    if (!schedule) { setError("Choose a time."); return; }
    const chosen = Object.entries(files).map(([chart, format]) => ({ chart, format, values }));
    const input: ReportSubscriptionInput = {
      schedule,
      timeZone: effectiveZone,
      attachments: { pdf: pdf ? { paper, orientation } : null, charts: chosen },
      filters: filters && filters.length ? filters : null,
      params: existing ? existing.params : parseJson(readState.params),
      grain: existing ? existing.grain : (readState.grain ?? null),
      skipWhenEmpty: skip,
      recipients: recipients.map((r) => (r.userId ? { userId: r.userId } : { email: r.email as string })),
    };
    setError(null);
    // The route's own sentence on a refusal: an outside domain not listed, 51
    // recipients, a user who is not active. The form stays open.
    const saved = await saveAction.run(existing?.id ?? null, input);
    if (saved) onSaved();
  }

  const summary = schedule ? describeSchedule(schedule, effectiveZone) : "";
  const filterCount = filters?.length ?? 0;
  // Owner QA 2026-11-16: the When fields sit in two even columns, labels on
  // the left above their controls; with three fields the zone takes the row.
  const whenFields = kind === "hourly" || kind === "daily" ? 3 : 4;

  return (
    <div className="flex min-w-0 flex-col gap-6">
      <FormGroup title="When">
        <div className="grid grid-cols-[minmax(0,1fr)] gap-3 sm:grid-cols-2">
          <FormField id={`${ids}-repeat`} label="Repeat">
            <SelectField id={`${ids}-repeat`} className="w-full" value={kind} options={REPEAT_OPTIONS}
              onValueChange={(next) => setKind(next as Kind)} />
          </FormField>
          {kind === "weekly" ? (
            <FormField id={`${ids}-weekday`} label="Day">
              <SelectField id={`${ids}-weekday`} className="w-full" value={String(weekday)} options={WEEKDAY_OPTIONS}
                onValueChange={(next) => setWeekday(Number(next))} />
            </FormField>
          ) : null}
          {kind === "monthly" ? (
            <FormField id={`${ids}-month-day`} label="Day of the month">
              <SelectField id={`${ids}-month-day`} className="w-full" value={String(monthDay)} options={MONTH_DAY_OPTIONS}
                onValueChange={(next) => setMonthDay(next === "last" ? "last" : Number(next))} />
            </FormField>
          ) : null}
          {kind === "hourly" ? (
            <FormField id={`${ids}-minute`} label="Minutes past the hour">
              <Input id={`${ids}-minute`} type="number" inputMode="numeric" min={0} max={59} value={minute}
                onChange={(e) => setMinute(Math.min(59, Math.max(0, Number(e.target.value) || 0)))} className="tabular-nums" />
            </FormField>
          ) : (
            <FormField id={`${ids}-time`} label="Time">
              <Input id={`${ids}-time`} type="time" value={time} onChange={(e) => setTime(e.target.value)} className="tabular-nums" />
            </FormField>
          )}
          <FormField id={`${ids}-zone`} label="Time zone" className={cn(whenFields === 3 && "sm:col-span-2")}>
            <TimeZoneSelect id={`${ids}-zone`} value={effectiveZone} onValueChange={setZone} />
          </FormField>
        </div>
        {summary ? (
          <p className="flex min-w-0 items-center gap-1.5 text-sm text-muted-foreground" aria-live="polite">
            <CalendarClock className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 truncate">{summary}</span>
          </p>
        ) : null}
      </FormGroup>

      <FormGroup title="What to send">
        <SwitchRow
          label="The dashboard as a PDF"
          hint={!pdfSupported ? "The PDF service is not set up on this deployment." : null}
          checked={pdf}
          disabled={!pdfSupported}
          onCheckedChange={setPdf}
        />
        {pdf ? (
          <div className="grid grid-cols-2 gap-3">
            <FormField id={`${ids}-paper`} label="Paper">
              <SelectField id={`${ids}-paper`} className="w-full" value={paper} options={PAPER_OPTIONS} onValueChange={setPaper} />
            </FormField>
            <FormField id={`${ids}-orientation`} label="Orientation">
              <SelectField id={`${ids}-orientation`} className="w-full" value={orientation} options={ORIENTATION_OPTIONS} onValueChange={setOrientation} />
            </FormField>
          </div>
        ) : null}
        {charts.length > 0 || Object.keys(files).length > 0 ? (
          <ChartFiles id={`${ids}-files`} charts={charts} files={files} onChange={setFiles} />
        ) : null}
        {Object.keys(files).length > 0 ? (
          <FormField id={`${ids}-values`} label="Values in the files">
            <SelectField id={`${ids}-values`} className="w-full" value={values} options={VALUES_OPTIONS} onValueChange={setValues} />
          </FormField>
        ) : null}
        {/* RPT-1 QA fix Q3: single-value tiles always have a value, so they are not judged. */}
        <SwitchRow
          label="Skip when empty"
          hint={charts.some((c) => c.grouped)
            ? "Send nothing when every chart has no rows."
            : <>Always sent: every chart here has a single value <HelpHint>{ALWAYS_SENT_SENTENCE}</HelpHint></>}
          checked={skip}
          onCheckedChange={setSkip}
        />
      </FormGroup>

      <FormGroup title="Filters">
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5">
          <p className="min-w-0 flex-1 basis-56 text-sm">
            {filterCount ? `${filterCount} ${filterCount === 1 ? "filter" : "filters"} will be applied to this report.` : "No filters: the dashboard's own."}
          </p>
          <div className="-mr-2 flex shrink-0 gap-0.5">
            <Button variant="ghost" size="xs" disabled={current.length === 0} onClick={() => setFilters(current)}>Use the filters on screen</Button>
            <Button variant="ghost" size="xs" disabled={filterCount === 0} onClick={() => setFilters(null)}>Clear</Button>
          </div>
        </div>
      </FormGroup>

      <FormGroup title="Recipients" headingId={`${ids}-recipients`}>
        {/* Feature decision 1: said above the field, before anyone is added. */}
        <FormHint className="-mt-1">
          <span id={`${ids}-as-you`} className="min-w-0 truncate">Everyone sees the data as you see it</span>
          <HelpHint>{AS_CREATOR_SENTENCE}</HelpHint>
        </FormHint>
        <ReportRecipientsField
          id={`${ids}-recipient-input`}
          labelledBy={`${ids}-recipients`}
          describedBy={`${ids}-as-you`}
          value={recipients}
          onChange={setRecipients}
        />
      </FormGroup>

      {shownError ? <p role="alert" className="text-sm text-destructive">{shownError}</p> : null}
      <DialogFooter>
        <Button size="sm" disabled={busy} onClick={() => void save()}>
          {busy ? <Loader2 className="size-3.5 animate-spin" aria-hidden /> : null}
          {busy ? "Saving…" : existing ? "Save changes" : "Subscribe"}
        </Button>
      </DialogFooter>
    </div>
  );
}

/**
 * The chart files a report attaches (owner QA 2026-11-16: a select per chart,
 * each saying "Not attached", was a heavy stack). One searchable field adds a
 * chart's data; each chart added is a quiet row with its format and a remove.
 * A chart the subscription attaches that is no longer on the dashboard keeps
 * its row, so it can be seen and left out.
 */
function ChartFiles({ id, charts, files, onChange }: {
  readonly id: string;
  readonly charts: readonly DashboardChart[];
  readonly files: Record<string, FileFormat>;
  readonly onChange: (next: Record<string, FileFormat>) => void;
}) {
  const chosen = charts.filter((c) => c.id in files);
  const gone = Object.keys(files).filter((chart) => !charts.some((c) => c.id === chart));
  const rows = [...chosen.map((c) => ({ id: c.id, title: c.title })), ...gone.map((chart) => ({ id: chart, title: "A chart no longer on this dashboard" }))];
  function setAttached(next: DashboardChart[]) {
    const kept = Object.fromEntries(Object.entries(files).filter(([chart]) => gone.includes(chart) || next.some((c) => c.id === chart)));
    for (const c of next) if (!(c.id in kept)) kept[c.id] = "xlsx";
    onChange(kept);
  }
  return (
    <FormField
      id={id}
      label="Chart data"
      hint={<>Up to 10 files, 20 MB in all <HelpHint>A file that does not fit is left out, and the email says so.</HelpHint></>}
    >
      <Combobox
        multiple
        items={[...charts]}
        value={chosen}
        onValueChange={(next: DashboardChart[]) => setAttached(next)}
        isItemEqualToValue={(a: DashboardChart, b: DashboardChart) => a.id === b.id}
        itemToStringLabel={(c: DashboardChart) => c.title}
      >
        <ComboboxInput id={id} className="w-full" placeholder={chosen.length ? "Add another chart" : "Add a chart's data"} autoComplete="off" />
        <ComboboxContent>
          <ComboboxEmpty>No chart matches.</ComboboxEmpty>
          <ComboboxList>
            {(c: DashboardChart) => <ComboboxItem key={c.id} value={c}><span className="min-w-0 truncate">{c.title}</span></ComboboxItem>}
          </ComboboxList>
        </ComboboxContent>
      </Combobox>
      {rows.length ? (
        <ul aria-label="Attached chart data" className="-mx-2 flex min-w-0 flex-col">
          {rows.map((r) => (
            <li key={r.id} className="flex h-9 min-w-0 items-center gap-2.5 rounded-lg pr-1 pl-2 transition-colors duration-150 ease-out hover:bg-muted/60 motion-reduce:transition-none">
              <FileSpreadsheet className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
              <span className="min-w-0 flex-1 truncate text-sm" title={r.title}>{r.title}</span>
              <FormatToggle label={`Format for ${r.title}`} value={files[r.id] ?? "xlsx"} onChange={(f) => onChange({ ...files, [r.id]: f })} />
              <Button variant="ghost" size="icon-xs" aria-label={`Leave out ${r.title}`} className="text-muted-foreground"
                onClick={() => onChange(Object.fromEntries(Object.entries(files).filter(([chart]) => chart !== r.id)))}>
                <X aria-hidden />
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
    </FormField>
  );
}

/** Excel or CSV for one attached chart: a two-choice radio group, the arrow keys move between them. */
function FormatToggle({ label, value, onChange }: {
  readonly label: string;
  readonly value: FileFormat;
  readonly onChange: (next: FileFormat) => void;
}) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  return (
    <div role="radiogroup" aria-label={label} className="flex shrink-0 rounded-md bg-muted p-0.5">
      {FILE_FORMATS.map((f, i) => {
        const on = f.value === value;
        return (
          <button
            key={f.value}
            ref={(el) => { refs.current[i] = el; }}
            type="button"
            role="radio"
            aria-checked={on}
            tabIndex={on ? 0 : -1}
            onClick={() => onChange(f.value)}
            onKeyDown={(e) => {
              if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
              e.preventDefault();
              const next = FILE_FORMATS[(i + 1) % FILE_FORMATS.length];
              onChange(next.value);
              refs.current[(i + 1) % FILE_FORMATS.length]?.focus();
            }}
            className={cn(
              "h-6 rounded-[5px] px-2 text-xs font-medium outline-none transition-colors duration-150 ease-out focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
              on ? "bg-background text-foreground shadow-sm dark:bg-input" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {f.label}
          </button>
        );
      })}
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
