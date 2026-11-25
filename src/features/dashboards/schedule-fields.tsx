"use client";

import { FormField } from "@/components/patterns/form-fields";
import { SelectField } from "@/components/patterns/select-field";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { ReportSchedule } from "@/services/contracts/reports";
import { clock, WEEKDAYS } from "./report-words";
import { TimeZoneSelect } from "@/components/patterns/time-zone-select";

/**
 * The "when is it sent" fields of a report subscription and of a dashboard
 * digest (`BI-65` decision 5): Repeat (every hour, day, week, month), the day,
 * the time and the time zone. One component for both, so the two forms cannot
 * drift (`AGENTS.md` rule 4); the schedule it makes is the same
 * `ReportSchedule` the API reads for both.
 *
 * Controlled by a [`ScheduleDraft`]: the form's own state, which keeps a
 * choice the person made for a kind they have since left (the minute of an
 * hourly schedule, the weekday of a weekly one), so flipping Repeat back and
 * forth loses nothing. [`scheduleOfDraft`] reads the schedule out of it.
 */

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

type Kind = ReportSchedule["kind"];

/** What the schedule fields hold: every choice, so a kind left and come back to keeps its own. */
export type ScheduleDraft = {
  readonly kind: Kind;
  /** The minute past the hour of an hourly schedule. */
  readonly minute: number;
  /** `HH:MM` of a daily, weekly or monthly one; what the time input holds. */
  readonly time: string;
  /** ISO weekday, 1 (Monday) to 7. */
  readonly weekday: number;
  readonly monthDay: number | "last";
  /** The IANA zone; empty means "the report time zone" (see [`zoneOfDraft`]). */
  readonly zone: string;
};

/** Every Monday at 08:00, what a new subscription or digest starts with. */
export const DEFAULT_SCHEDULE: ReportSchedule = { kind: "weekly", weekday: 1, hour: 8, minute: 0 };

/**
 * A digest saved before schedules existed is sent once a day at 08:00 in the
 * report time zone (`BI-65` decision 3), so that is what its form opens with:
 * saving it unchanged keeps its behaviour.
 */
export const LEGACY_DIGEST_SCHEDULE: ReportSchedule = { kind: "daily", hour: 8, minute: 0 };

/** The draft of `schedule` read in `zone` (empty: the report time zone). */
export function draftFromSchedule(schedule: ReportSchedule, zone: string): ScheduleDraft {
  return {
    kind: schedule.kind,
    minute: schedule.minute,
    time: "hour" in schedule ? clock(schedule.hour, schedule.minute) : "08:00",
    weekday: schedule.kind === "weekly" ? schedule.weekday : 1,
    monthDay: schedule.kind === "monthly" ? schedule.monthDay : 1,
    zone,
  };
}

/** The schedule the draft says, or `null` when its time is not a time yet. */
export function scheduleOfDraft(d: ScheduleDraft): ReportSchedule | null {
  if (d.kind === "hourly") return { kind: "hourly", minute: d.minute };
  const [hour, minute] = d.time.split(":").map((n) => Number.parseInt(n, 10));
  if (!Number.isFinite(hour) || !Number.isFinite(minute)) return null;
  if (d.kind === "daily") return { kind: "daily", hour, minute };
  if (d.kind === "weekly") return { kind: "weekly", weekday: d.weekday, hour, minute };
  return { kind: "monthly", monthDay: d.monthDay, hour, minute };
}

/** The zone the draft is read in: its own, else `reportZone`. */
export function zoneOfDraft(d: ScheduleDraft, reportZone: string): string {
  return d.zone.trim() || reportZone;
}

/**
 * Owner QA 2026-11-16: the fields sit in two even columns, labels above their
 * controls; with three fields (an hourly or a daily schedule) the zone takes
 * the second row whole.
 */
export function ScheduleFields({
  id, draft, onChange, reportZone,
}: {
  /** Prefix of the field ids, so two of these on a page do not share labels. */
  readonly id: string;
  readonly draft: ScheduleDraft;
  readonly onChange: (next: ScheduleDraft) => void;
  /** The report time zone: what an empty zone reads as. */
  readonly reportZone: string;
}) {
  const set = (patch: Partial<ScheduleDraft>) => onChange({ ...draft, ...patch });
  const threeFields = draft.kind === "hourly" || draft.kind === "daily";
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-3 sm:grid-cols-2">
      <FormField id={`${id}-repeat`} label="Repeat">
        <SelectField id={`${id}-repeat`} className="w-full" value={draft.kind} options={REPEAT_OPTIONS}
          onValueChange={(next) => set({ kind: next as Kind })} />
      </FormField>
      {draft.kind === "weekly" ? (
        <FormField id={`${id}-weekday`} label="Day">
          <SelectField id={`${id}-weekday`} className="w-full" value={String(draft.weekday)} options={WEEKDAY_OPTIONS}
            onValueChange={(next) => set({ weekday: Number(next) })} />
        </FormField>
      ) : null}
      {draft.kind === "monthly" ? (
        <FormField id={`${id}-month-day`} label="Day of the month">
          <SelectField id={`${id}-month-day`} className="w-full" value={String(draft.monthDay)} options={MONTH_DAY_OPTIONS}
            onValueChange={(next) => set({ monthDay: next === "last" ? "last" : Number(next) })} />
        </FormField>
      ) : null}
      {draft.kind === "hourly" ? (
        <FormField id={`${id}-minute`} label="Minutes past the hour">
          <Input id={`${id}-minute`} type="number" inputMode="numeric" min={0} max={59} value={draft.minute}
            onChange={(e) => set({ minute: Math.min(59, Math.max(0, Number(e.target.value) || 0)) })} className="tabular-nums" />
        </FormField>
      ) : (
        <FormField id={`${id}-time`} label="Time">
          <Input id={`${id}-time`} type="time" value={draft.time} onChange={(e) => set({ time: e.target.value })} className="tabular-nums" />
        </FormField>
      )}
      <FormField id={`${id}-zone`} label="Time zone" className={cn(threeFields && "sm:col-span-2")}>
        <TimeZoneSelect id={`${id}-zone`} value={zoneOfDraft(draft, reportZone)} onValueChange={(zone) => set({ zone })} />
      </FormField>
    </div>
  );
}
