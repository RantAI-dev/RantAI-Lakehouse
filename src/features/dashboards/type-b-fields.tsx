"use client";

import * as React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { AUTO_HISTOGRAM_BINS, MAX_ALT, MAX_BANDS, MAX_GANTT_BARS, MAX_HISTOGRAM_COUNT, urlProblem, type TypeDraft } from "@/lib/type-b";
import { columnItems } from "./builder-sections";

/**
 * The controls the progress, histogram, bullet, image and web kinds add to the
 * chart builder (`BI-16` part B). Each is a controlled piece of the builder's
 * `TypeDraft`; the builder sends only what the chosen kind takes
 * (`lib/type-b.typePayload`) and the server checks every limit again.
 */

type Change = (patch: Partial<TypeDraft>) => void;

const GOAL_MODES = { number: "A number", measure: "A second value" } as const;

/** The goal of a progress bar, or the (optional) target of a bullet graph: a number, or a second value of the same source. */
export function GoalFields({
  kind, draft, onChange, columns, fieldLabel, disabled, placeholder,
}: {
  readonly kind: "progress" | "bullet";
  readonly draft: TypeDraft;
  readonly onChange: Change;
  /** The value columns the second value can be. */
  readonly columns: readonly string[];
  readonly fieldLabel: (name: string) => React.ReactNode;
  readonly disabled: boolean;
  readonly placeholder: string;
}) {
  const word = kind === "progress" ? "Goal" : "Target";
  return (
    <div className="grid gap-1.5">
      <Label>
        {word} {kind === "bullet" ? <span className="font-normal text-muted-foreground">(optional)</span> : <span aria-hidden className="text-destructive">*</span>}
      </Label>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Select value={draft.goalMode} items={GOAL_MODES} onValueChange={(v) => onChange({ goalMode: v === "measure" ? "measure" : "number" })}>
          <SelectTrigger className="w-full" aria-label={`${word} is`}><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="number">{GOAL_MODES.number}</SelectItem>
            <SelectItem value="measure">{GOAL_MODES.measure}</SelectItem>
          </SelectContent>
        </Select>
        {draft.goalMode === "measure" ? (
          <Select value={draft.goalMeasure} items={columnItems(columns, fieldLabel, draft.goalMeasure)} onValueChange={(v) => onChange({ goalMeasure: v ?? "" })} disabled={disabled}>
            <SelectTrigger className="w-full" aria-label={`${word} column`}><SelectValue placeholder={disabled ? placeholder : "pick a column"} /></SelectTrigger>
            <SelectContent>{columns.map((c) => <SelectItem key={c} value={c}>{fieldLabel(c)}</SelectItem>)}</SelectContent>
          </Select>
        ) : (
          <Input type="number" aria-label={`${word} number`} value={draft.goalNumber} onChange={(e) => onChange({ goalNumber: e.target.value })} placeholder="e.g. 1000000" />
        )}
      </div>
      {draft.goalMode === "measure" ? (
        <p className="text-xs text-muted-foreground">The second value is aggregated like the first, over the same rows and filters.</p>
      ) : null}
    </div>
  );
}

/** Up to three rising upper limits of a bullet graph's bands. */
export function BandFields({ draft, onChange }: { readonly draft: TypeDraft; readonly onChange: Change }) {
  const slots = Array.from({ length: MAX_BANDS }, (_, i) => draft.bands[i] ?? "");
  return (
    <div className="grid gap-1.5">
      <Label>Bands <span className="font-normal text-muted-foreground">(optional, rising upper limits)</span></Label>
      <div className="grid grid-cols-3 gap-3">
        {slots.map((value, i) => (
          <Input
            key={i} type="number" aria-label={`Band ${i + 1} limit`} value={value} placeholder={["poor", "fair", "good"][i]}
            onChange={(e) => {
              const next = [...slots];
              next[i] = e.target.value;
              onChange({ bands: next });
            }}
          />
        ))}
      </div>
    </div>
  );
}

const HIST_MODES = { auto: `Automatic (about ${AUTO_HISTOGRAM_BINS})`, count: "A number of ranges", width: "A width" } as const;

/** How a histogram cuts its column: automatic (the nice width nearest 20 ranges), exactly N ranges, or a width (BI-18 part C review fix (SHOULD-FIX) R21). */
export function HistogramFields({ draft, onChange }: { readonly draft: TypeDraft; readonly onChange: Change }) {
  return (
    <div className="grid gap-1.5">
      <Label>Ranges</Label>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Select value={draft.histMode} items={HIST_MODES} onValueChange={(v) => onChange({ histMode: v === "count" || v === "width" ? v : "auto" })}>
          <SelectTrigger className="w-full" aria-label="Ranges"><SelectValue /></SelectTrigger>
          <SelectContent>
            {Object.entries(HIST_MODES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
          </SelectContent>
        </Select>
        {draft.histMode === "count" ? (
          <Input type="number" min={1} max={MAX_HISTOGRAM_COUNT} aria-label="Number of ranges" value={draft.histCount} onChange={(e) => onChange({ histCount: e.target.value })} />
        ) : draft.histMode === "width" ? (
          <Input type="number" min={0} step="any" aria-label="Range width" value={draft.histWidth} onChange={(e) => onChange({ histWidth: e.target.value })} placeholder="e.g. 500" />
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground">Counts the rows in each range of the number column. At most {MAX_HISTOGRAM_COUNT} ranges.</p>
    </div>
  );
}

const NO_PROGRESS = "__none__";

/**
 * The start, end and optional progress columns of a Gantt chart (`BI-16` part
 * C1, plan D1). Start and end are date or date-time columns; progress is a
 * number column read as 0 to 100 percent. The task (Task column) and the group
 * (Group) are the builder's own dimension and breakdown fields.
 */
export function GanttFields({
  draft, onChange, dateColumns, numberColumns, fieldLabel, disabled, placeholder,
}: {
  readonly draft: TypeDraft;
  readonly onChange: Change;
  readonly dateColumns: readonly string[];
  readonly numberColumns: readonly string[];
  readonly fieldLabel: (name: string) => React.ReactNode;
  readonly disabled: boolean;
  readonly placeholder: string;
}) {
  const pick = (label: string, value: string, set: (v: string) => void, required: boolean) => (
    <div className="grid content-start gap-1.5">
      <Label>{label} {required ? <span aria-hidden className="text-destructive">*</span> : <span className="font-normal text-muted-foreground">(optional, 0 to 100)</span>}</Label>
      <Select
        value={value || (required ? "" : NO_PROGRESS)}
        items={{ ...(required ? {} : { [NO_PROGRESS]: "— no progress —" }), ...columnItems(required ? dateColumns : numberColumns, fieldLabel, value) }}
        onValueChange={(v) => set(!v || v === NO_PROGRESS ? "" : v)} disabled={disabled}
      >
        <SelectTrigger className="w-full" aria-label={label}><SelectValue placeholder={disabled ? placeholder : "pick a column"} /></SelectTrigger>
        <SelectContent>
          {required ? null : <SelectItem value={NO_PROGRESS}>— no progress —</SelectItem>}
          {(required ? dateColumns : numberColumns).map((c) => <SelectItem key={c} value={c}>{fieldLabel(c)}</SelectItem>)}
        </SelectContent>
      </Select>
    </div>
  );
  return (
    <div className="grid gap-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {pick("Start column", draft.ganttStart, (v) => onChange({ ganttStart: v }), true)}
        {pick("End column", draft.ganttEnd, (v) => onChange({ ganttEnd: v }), true)}
      </div>
      {pick("Progress column", draft.ganttProgress, (v) => onChange({ ganttProgress: v }), false)}
      <p className="text-xs text-muted-foreground">One bar per task and group, from its earliest start to its latest end. At most {MAX_GANTT_BARS.toLocaleString("en-US")} bars.</p>
    </div>
  );
}

/** The address (and, for an image, the description) of an image or web-page card, with the sites it may come from. */
export function CardFields({
  kind, draft, onChange, sites,
}: {
  readonly kind: "image" | "web";
  readonly draft: TypeDraft;
  readonly onChange: Change;
  readonly sites: readonly string[];
}) {
  const problem = urlProblem(draft.url, sites);
  return (
    <>
      <div className="grid gap-1.5">
        <Label htmlFor="ch-url">Address <span aria-hidden className="text-destructive">*</span></Label>
        <Input
          id="ch-url" type="url" inputMode="url" autoComplete="off" value={draft.url} onChange={(e) => onChange({ url: e.target.value })}
          placeholder={sites[0] ? `${sites[0]}/${kind === "image" ? "logo.png" : "status"}` : "https://…"}
          aria-invalid={problem ? true : undefined} aria-describedby="ch-url-sites"
        />
        {problem ? <p role="alert" className="text-xs text-destructive">{problem}</p> : null}
        <div id="ch-url-sites" className="text-xs text-muted-foreground">
          {sites.length ? (
            <>
              <p>Allowed sites:</p>
              <ul aria-label="Allowed content sites" className="font-mono">
                {sites.map((s) => <li key={s}>{s}</li>)}
              </ul>
            </>
          ) : (
            <p>No site is allowed yet. An administrator lists the sites {kind === "image" ? "images" : "web pages"} may come from in Admin &gt; Settings.</p>
          )}
        </div>
      </div>
      {kind === "image" ? (
        <div className="grid gap-1.5">
          <Label htmlFor="ch-alt">Description <span aria-hidden className="text-destructive">*</span></Label>
          <Input id="ch-alt" value={draft.alt} maxLength={MAX_ALT} onChange={(e) => onChange({ alt: e.target.value })} placeholder="What the picture shows, for screen readers" />
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">Many sites refuse to be shown inside another page; the card then stays blank and offers “Open in a new tab”.</p>
      )}
    </>
  );
}
