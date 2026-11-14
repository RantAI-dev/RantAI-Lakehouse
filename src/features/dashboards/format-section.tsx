"use client";

import * as React from "react";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingField, SettingGroup, SettingSwitch } from "./builder-sections";
import {
  BUILTIN_PALETTES, DATE_STYLES, MAX_ANNOTATIONS, MAX_AFFIX_LEN, MAX_AXIS_TITLE_LEN, MAX_CONDITIONAL_RULES, MAX_DECIMALS, MAX_NOTE_LEN,
  type Annotation, type AxisStyle, type CompareOp, type CondRule, type CondRuleTest, type DateFormat, type StyleFields, type ValueFormat,
} from "@/lib/chart-style";
import { MAX_CLUSTERS, MIN_CLUSTERS } from "@/lib/chart-style";
import type { FormatRules } from "@/lib/style-draft";
import { ColourInput } from "./colour-input";

/**
 * Section 5 of the chart builder, "Format" (`BI-17` part A, T7): number and date formats, colours,
 * axes, data labels, notes, tooltip fields. Reference lines have their own editor, placed in this
 * section by the builder. A group the chart's kind does not take is not drawn (`formatRules`), so
 * the form never offers what the server would refuse.
 */

// BI-58 review round 1 (SHOULD-FIX 3): "Automatic" wherever the choice means the chart's own
// default ("As it is now" and "Time as is" read oddly to the owner).
const STYLE_LABELS: Record<string, string> = {
  auto: "Automatic", number: "Number", percent: "Percent", currency: "Currency", scientific: "Scientific", compact: "Compact (1.2 K)",
  currencyCompact: "Currency, compact (1.2 M)",
};
// QA LH-BI-1-06 (BI-55, D12): a compact amount of money is one choice of its
// own, so the list says that Compact takes a currency code. `FORMAT_STYLES`
// mirrors the server and is not changed; this list is what the editor offers.
const STYLE_CHOICES = ["auto", "number", "percent", "currency", "currencyCompact", "scientific", "compact"] as const;
const DATE_LABELS: Record<string, string> = {
  auto: "Automatic", short: "15/10/26", medium: "15 Oct 2026", long: "15 October 2026", iso: "2026-10-15", monthYear: "October 2026",
};
const TIME_LABELS: Record<string, string> = { locale: "Automatic", "12": "12-hour", "24": "24-hour" };
const PALETTE_LABELS: Record<string, string> = {
  default: "Default", bright: "Bright", muted: "Muted", "sequential-blue": "Sequential blue", "diverging-red-blue": "Diverging red-blue", company: "Company palette",
};
const COMPARE_LABELS: Record<CompareOp, string> = { lt: "is below", le: "is at most", gt: "is above", ge: "is at least", eq: "equals", ne: "does not equal" };
const RULE_LABELS: Record<CondRuleTest["type"], string> = { compare: "Compare", between: "Between", contains: "Contains", range: "Colour range" };
const ROTATIONS: Record<string, string> = { "": "Automatic", "0": "Flat", "45": "45°", "90": "Upright" };
const ALL_VALUES = "__all__";

function num(text: string): number | undefined {
  return text.trim() === "" || !Number.isFinite(Number(text)) ? undefined : Number(text);
}

/** A sub-heading inside a group (an axis, a note, a rule), with its remove button when it has one. */
function ItemHeading({ children, onRemove, removeLabel }: { readonly children: React.ReactNode; readonly onRemove?: () => void; readonly removeLabel?: string }) {
  return (
    <div className="flex min-h-6 items-center justify-between gap-2 pt-2">
      <p className="text-sm font-semibold text-foreground">{children}</p>
      {onRemove ? <Button type="button" size="icon-xs" variant="ghost" aria-label={removeLabel} onClick={onRemove}><X /></Button> : null}
    </div>
  );
}

/** One number format: style, decimals, thousands, currency, prefix and suffix. */
export function ValueFormatEditor({ value, onChange, label }: {
  readonly value: ValueFormat | undefined;
  readonly onChange: (next: ValueFormat) => void;
  readonly label: string;
}) {
  const f = value ?? {};
  const style = f.style ?? "auto";
  const set = (patch: Partial<ValueFormat>) => onChange({ ...f, ...patch });
  const currencyStyle = style === "currency" || style === "compact";
  // A stored compact amount (a compact style with a currency) is shown as the
  // "Currency, compact" choice.
  const choice = style === "compact" && f.currency ? "currencyCompact" : style;
  const choose = (v: string | null) => {
    if (v === "currencyCompact") set({ style: "compact", currency: f.currency ?? "IDR" });
    else if (v === "compact") set({ style: "compact", currency: undefined, currencyDisplay: undefined });
    else set({ style: (v ?? "auto") as ValueFormat["style"] });
  };
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <SettingField label="Style">
        {(id) => (
          <Select value={choice} items={STYLE_LABELS} onValueChange={choose}>
            <SelectTrigger id={id} className="w-full" aria-label={`${label} style`}><SelectValue /></SelectTrigger>
            <SelectContent>{STYLE_CHOICES.map((s) => <SelectItem key={s} value={s}>{STYLE_LABELS[s]}</SelectItem>)}</SelectContent>
          </Select>
        )}
      </SettingField>
      <div className={currencyStyle ? "grid grid-cols-2 gap-3" : "grid grid-cols-[minmax(0,1fr)]"}>
        <SettingField label="Decimals">
          {(id) => <Input id={id} type="number" min={0} max={MAX_DECIMALS} value={f.decimals ?? ""} placeholder="Auto" aria-label={`${label} decimals`} onChange={(e) => set({ decimals: num(e.target.value) })} />}
        </SettingField>
        {currencyStyle ? (
          <SettingField label="Currency">
            {(id) => <Input id={id} className="uppercase" maxLength={3} value={f.currency ?? ""} placeholder="IDR" aria-label={`${label} currency code`} onChange={(e) => set({ currency: e.target.value.toUpperCase() || undefined })} />}
          </SettingField>
        ) : null}
      </div>
      {currencyStyle ? (
        <SettingField label="Currency shown as">
          {(id) => (
            <Select value={f.currencyDisplay ?? "symbol"} items={{ symbol: "Symbol (Rp)", code: "Code (IDR)" }} onValueChange={(v) => set({ currencyDisplay: v === "code" ? "code" : "symbol" })}>
              <SelectTrigger id={id} className="w-full" aria-label={`${label} currency display`}><SelectValue /></SelectTrigger>
              <SelectContent><SelectItem value="symbol">Symbol (Rp)</SelectItem><SelectItem value="code">Code (IDR)</SelectItem></SelectContent>
            </Select>
          )}
        </SettingField>
      ) : null}
      <div className="grid grid-cols-2 gap-3">
        <SettingField label="Prefix">
          {(id) => <Input id={id} maxLength={MAX_AFFIX_LEN} value={f.prefix ?? ""} aria-label={`${label} prefix`} onChange={(e) => set({ prefix: e.target.value || undefined })} />}
        </SettingField>
        <SettingField label="Suffix">
          {(id) => <Input id={id} maxLength={MAX_AFFIX_LEN} value={f.suffix ?? ""} aria-label={`${label} suffix`} onChange={(e) => set({ suffix: e.target.value || undefined })} />}
        </SettingField>
      </div>
      <SettingSwitch label="Thousands separator" ariaLabel={`${label} thousands separator`} checked={f.thousands !== false} onCheckedChange={(on) => set({ thousands: on ? undefined : false })} />
    </div>
  );
}

function DateFormatEditor({ value, onChange }: { readonly value: DateFormat | undefined; readonly onChange: (next: DateFormat) => void }) {
  const f = value ?? {};
  const set = (patch: Partial<DateFormat>) => onChange({ ...f, ...patch });
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <SettingField label="Date style">
        {(id) => (
          <Select value={f.style ?? "auto"} items={DATE_LABELS} onValueChange={(v) => set({ style: (v ?? "auto") as DateFormat["style"] })}>
            <SelectTrigger id={id} className="w-full" aria-label="Date style"><SelectValue /></SelectTrigger>
            <SelectContent>{DATE_STYLES.map((s) => <SelectItem key={s} value={s}>{DATE_LABELS[s]}</SelectItem>)}</SelectContent>
          </Select>
        )}
      </SettingField>
      <SettingField label="Time">
        {(id) => (
          <Select value={f.hour12 === undefined ? "locale" : f.hour12 ? "12" : "24"} items={TIME_LABELS}
            onValueChange={(v) => set({ hour12: v === "12" ? true : v === "24" ? false : undefined })}>
            <SelectTrigger id={id} className="w-full" aria-label="Time format"><SelectValue /></SelectTrigger>
            <SelectContent>{Object.entries(TIME_LABELS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
          </Select>
        )}
      </SettingField>
      <SettingSwitch label="Short month and day names (Oct, Mon)" ariaLabel="Short month and day names" checked={f.abbreviate === true} onCheckedChange={(on) => set({ abbreviate: on ? true : undefined })} />
    </div>
  );
}

function AxisEditor({ title, value, onChange, canLog, rotate }: {
  readonly title: string;
  readonly value: AxisStyle | undefined;
  readonly onChange: (next: AxisStyle) => void;
  readonly canLog: boolean;
  readonly rotate: boolean;
}) {
  const a = value ?? {};
  const set = (patch: Partial<AxisStyle>) => onChange({ ...a, ...patch });
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <ItemHeading>{title} axis</ItemHeading>
      <SettingField label="Title">
        {(id) => <Input id={id} maxLength={MAX_AXIS_TITLE_LEN} value={a.title ?? ""} aria-label={`${title} axis title`} onChange={(e) => set({ title: e.target.value || undefined })} />}
      </SettingField>
      {canLog ? (
        <div className="grid grid-cols-2 gap-3">
          <SettingField label="Minimum">
            {(id) => <Input id={id} type="number" value={a.min ?? ""} placeholder="Auto" aria-label={`${title} axis minimum`} onChange={(e) => set({ min: num(e.target.value) })} />}
          </SettingField>
          <SettingField label="Maximum">
            {(id) => <Input id={id} type="number" value={a.max ?? ""} placeholder="Auto" aria-label={`${title} axis maximum`} onChange={(e) => set({ max: num(e.target.value) })} />}
          </SettingField>
        </div>
      ) : null}
      {rotate ? (
        <SettingField label="Label angle">
          {(id) => (
            <Select value={a.rotate === undefined ? "" : String(a.rotate)} items={ROTATIONS} onValueChange={(v) => set({ rotate: v === "" || v == null ? undefined : (Number(v) as 0 | 45 | 90) })}>
              <SelectTrigger id={id} className="w-full" aria-label={`${title} axis label rotation`}><SelectValue /></SelectTrigger>
              <SelectContent>{Object.entries(ROTATIONS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
            </Select>
          )}
        </SettingField>
      ) : null}
      {canLog ? (
        <SettingSwitch label="Logarithmic scale" ariaLabel={`${title} axis logarithmic`} checked={a.log === true} onCheckedChange={(on) => set({ log: on ? true : undefined })} />
      ) : null}
      <SettingSwitch label="Show this axis" ariaLabel={`Show the ${title} axis`} checked={a.show !== false} onCheckedChange={(on) => set({ show: on ? undefined : false })} />
    </div>
  );
}

const NEW_RULE: CondRule = { column: "", rule: { type: "compare", op: "lt", value: 0 }, color: "#dc2626" };

/** One conditional rule of a table, pivot or KPI. */
function RuleRow({ rule, columns, index, onChange, onRemove }: {
  readonly rule: CondRule;
  readonly columns: readonly string[];
  readonly index: number;
  readonly onChange: (next: CondRule) => void;
  readonly onRemove: () => void;
}) {
  const t = rule.rule;
  const n = index + 1;
  const setTest = (test: CondRuleTest) => onChange(test.type === "range" ? { column: rule.column, rule: test } : { ...rule, rule: test, color: rule.color ?? "#dc2626" });
  const switchType = (type: CondRuleTest["type"]) => {
    if (type === "compare") setTest({ type, op: "lt", value: 0 });
    else if (type === "between") setTest({ type, from: 0, to: 0 });
    else if (type === "contains") setTest({ type, text: "" });
    else setTest({ type, colors: ["#fee2e2", "#dc2626"] });
  };
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <ItemHeading onRemove={onRemove} removeLabel={`Remove rule ${n}`}>Rule {n}</ItemHeading>
      <SettingField label="Column">
        {(id) => (
          <Select value={rule.column || "__none__"} items={{ __none__: "Pick a column", ...Object.fromEntries(columns.map((c) => [c, c])) }} onValueChange={(v) => onChange({ ...rule, column: v && v !== "__none__" ? v : "" })}>
            <SelectTrigger id={id} className="w-full" aria-label={`Rule ${n} column`}><SelectValue /></SelectTrigger>
            <SelectContent>{columns.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
          </Select>
        )}
      </SettingField>
      <SettingField label="When">
        {(id) => (
          <Select value={t.type} items={RULE_LABELS} onValueChange={(v) => switchType((v ?? "compare") as CondRuleTest["type"])}>
            <SelectTrigger id={id} className="w-full" aria-label={`Rule ${n} kind`}><SelectValue /></SelectTrigger>
            <SelectContent>{Object.entries(RULE_LABELS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
          </Select>
        )}
      </SettingField>
      {t.type === "compare" ? (
        <div className="grid grid-cols-2 gap-3">
          <SettingField label="Comparison">
            {(id) => (
              <Select value={t.op} items={COMPARE_LABELS} onValueChange={(v) => setTest({ ...t, op: (v ?? "lt") as CompareOp })}>
                <SelectTrigger id={id} className="w-full" aria-label={`Rule ${n} operator`}><SelectValue /></SelectTrigger>
                <SelectContent>{Object.entries(COMPARE_LABELS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
              </Select>
            )}
          </SettingField>
          <SettingField label="Value">
            {(id) => <Input id={id} type="number" value={Number.isFinite(t.value) ? t.value : ""} aria-label={`Rule ${n} value`} onChange={(e) => setTest({ ...t, value: num(e.target.value) ?? Number.NaN })} />}
          </SettingField>
        </div>
      ) : null}
      {t.type === "between" ? (
        <div className="grid grid-cols-2 gap-3">
          <SettingField label="From">
            {(id) => <Input id={id} type="number" value={Number.isFinite(t.from) ? t.from : ""} aria-label={`Rule ${n} from`} onChange={(e) => setTest({ ...t, from: num(e.target.value) ?? Number.NaN })} />}
          </SettingField>
          <SettingField label="To">
            {(id) => <Input id={id} type="number" value={Number.isFinite(t.to) ? t.to : ""} aria-label={`Rule ${n} to`} onChange={(e) => setTest({ ...t, to: num(e.target.value) ?? Number.NaN })} />}
          </SettingField>
        </div>
      ) : null}
      {t.type === "contains" ? (
        <SettingField label="Text">
          {(id) => <Input id={id} value={t.text} maxLength={80} aria-label={`Rule ${n} text`} onChange={(e) => setTest({ ...t, text: e.target.value })} />}
        </SettingField>
      ) : null}
      {t.type === "range" ? (
        <div className="grid gap-1.5">
          <p className="text-xs font-medium">Colours, lowest to highest</p>
          <div className="flex flex-wrap items-center gap-1.5">
            {t.colors.map((c, i) => (
              <ColourInput key={i} value={c} label={`Rule ${n} range colour ${i + 1}`} onChange={(next) => { if (next) setTest({ ...t, colors: t.colors.map((x, j) => (j === i ? next : x)) }); }} />
            ))}
            <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs"
              onClick={() => setTest({ ...t, colors: t.colors.length === 2 ? [t.colors[0]!, "#f59e0b", t.colors[1]!] : [t.colors[0]!, t.colors[2]!] })}>
              {t.colors.length === 2 ? "Add a middle colour" : "Remove the middle colour"}
            </Button>
          </div>
        </div>
      ) : (
        <div className="grid grid-cols-[auto_minmax(0,1fr)] items-end gap-3">
          <div className="grid gap-1.5">
            <p className="text-xs font-medium">Colour</p>
            <ColourInput value={rule.color} label={`Rule ${n} colour`} onChange={(c) => onChange({ ...rule, color: c })} fallback="#dc2626" />
          </div>
          <SettingField label="Colours">
            {(id) => (
              <Select value={rule.target ?? "cell"} items={{ cell: "The cell", text: "The text" }} onValueChange={(v) => onChange({ ...rule, target: v === "text" ? "text" : "cell" })}>
                <SelectTrigger id={id} className="w-full" aria-label={`Rule ${n} colours`}><SelectValue /></SelectTrigger>
                <SelectContent><SelectItem value="cell">The cell</SelectItem><SelectItem value="text">The text</SelectItem></SelectContent>
              </Select>
            )}
          </SettingField>
        </div>
      )}
    </div>
  );
}

/** Colours by rule for a table, a pivot or a KPI. */
export function ConditionalEditor({ value, onChange, columns, showLabel = true }: {
  readonly value: readonly CondRule[];
  readonly onChange: (next: CondRule[]) => void;
  readonly columns: readonly string[];
  /** False inside the Style tab, where the group's heading already says it. */
  readonly showLabel?: boolean;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-4">
      {showLabel ? <Label>Conditional colours</Label> : null}
      {value.map((rule, i) => (
        <RuleRow key={i} rule={rule} columns={columns} index={i}
          onChange={(next) => onChange(value.map((r, j) => (j === i ? next : r)))} onRemove={() => onChange(value.filter((_, j) => j !== i))} />
      ))}
      {value.length < MAX_CONDITIONAL_RULES ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="-ml-2 h-7 px-2 text-xs" onClick={() => onChange([...value, { ...NEW_RULE, column: columns.length === 1 ? columns[0]! : "" }])}>
            <Plus className="size-3" aria-hidden /> Add a rule
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/** 2 to 10 groups, or let the console choose (`BI-17` part B). */
const CLUSTER_ITEMS: Record<string, string> = {
  auto: "Automatic",
  ...Object.fromEntries(Array.from({ length: MAX_CLUSTERS - MIN_CLUSTERS + 1 }, (_, i) => [String(MIN_CLUSTERS + i), String(MIN_CLUSTERS + i)])),
};

/** The groups of section 5 other than the reference lines. */
export function FormatSection({ rules, style, onChange, valueNames, seriesNames, paletteValue, children }: {
  readonly rules: FormatRules;
  readonly style: StyleFields;
  readonly onChange: (next: StyleFields) => void;
  /** The result names of the chart's values (or a table's columns), for per-value formats, the second axis and the tooltip. */
  readonly valueNames: readonly string[];
  /** The columns a conditional rule can read. */
  readonly seriesNames?: readonly string[];
  /**
   * The palette the chart draws with: the one the person chose, or the one a
   * chart with no choice of its own draws with (`PR #199 review O SHOULD-FIX 4`).
   */
  readonly paletteValue: string;
  /** The reference lines editor, placed after the axes. */
  readonly children?: React.ReactNode;
}) {
  const [formatOf, setFormatOf] = React.useState(ALL_VALUES);
  const set = (patch: Partial<StyleFields>) => onChange({ ...style, ...patch });
  const current = formatOf === ALL_VALUES ? style.format : style.valueFormats?.[formatOf];
  const setFormat = (f: ValueFormat) => (formatOf === ALL_VALUES ? set({ format: f }) : set({ valueFormats: { ...style.valueFormats, [formatOf]: f } }));
  const colors = style.colors ?? {};
  const setColors = (patch: Partial<typeof colors>) => set({ colors: { ...colors, ...patch } });
  const axes = style.axes ?? {};
  const setAxes = (patch: Partial<typeof axes>) => set({ axes: { ...axes, ...patch } });
  const notes = style.annotations ?? [];
  const setNote = (i: number, patch: Partial<Annotation>) => set({ annotations: notes.map((a, j) => (j === i ? { ...a, ...patch } : a)) });
  const tooltip = style.tooltip?.fields ?? [];
  const right = axes.rightValues ?? [];
  const toggle = (list: readonly string[], name: string, on: boolean) => (on ? [...list, name] : list.filter((n) => n !== name));
  const pickedSeries = Object.entries(colors.series ?? {});
  const byValue = colors.byValue;
  return (
    // BI-58 review round 1 (SHOULD-FIX 3): groups under small headings, set apart by space, not
    // bordered boxes; every field labelled above it and as wide as the panel.
    <div className="grid grid-cols-[minmax(0,1fr)] gap-7">
      {rules.values ? (
        <SettingGroup title="Values">
          {valueNames.length > 1 ? (
            <SettingField label="Format of">
              {(id) => (
                <Select value={formatOf} items={{ [ALL_VALUES]: "All values", ...Object.fromEntries(valueNames.map((n) => [n, n])) }} onValueChange={(v) => setFormatOf(v ?? ALL_VALUES)}>
                  <SelectTrigger id={id} className="w-full" aria-label="Format of"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ALL_VALUES}>All values</SelectItem>
                    {valueNames.map((n) => <SelectItem key={n} value={n}>{n}</SelectItem>)}
                  </SelectContent>
                </Select>
              )}
            </SettingField>
          ) : null}
          <ValueFormatEditor value={current} onChange={setFormat} label="Number format" />
        </SettingGroup>
      ) : null}
      {rules.dates ? (
        <SettingGroup title="Dates"><DateFormatEditor value={style.dateFormat} onChange={(f) => set({ dateFormat: f })} /></SettingGroup>
      ) : null}
      {rules.colors ? (
        <SettingGroup title="Colours">
          {/* The palette the person chose is stored as it is, "default" included:
              it is a choice of its own, so a company palette does not take it
              (`PR #199 review O SHOULD-FIX 4`). */}
          <SettingField label="Palette">
            {(id) => (
              <Select value={paletteValue} items={PALETTE_LABELS} onValueChange={(v) => setColors({ palette: v || undefined })}>
                <SelectTrigger id={id} className="w-full" aria-label="Palette"><SelectValue /></SelectTrigger>
                <SelectContent>{[...BUILTIN_PALETTES, "company"].map((p) => <SelectItem key={p} value={p}>{PALETTE_LABELS[p]}</SelectItem>)}</SelectContent>
              </Select>
            )}
          </SettingField>
          <div className="grid gap-1.5">
            <p className="text-xs font-medium">Colour of a series</p>
            {pickedSeries.map(([name, colour]) => (
              <div key={name} className="flex min-w-0 items-center gap-2">
                <ColourInput value={colour} label={`Colour of ${name}`} onChange={(c) => {
                  const next = { ...colors.series };
                  if (c) next[name] = c; else delete next[name];
                  setColors({ series: next });
                }} />
                <span className="min-w-0 truncate text-sm" title={name}>{name}</span>
              </div>
            ))}
            <SeriesAdder names={valueNames} taken={pickedSeries.map(([n]) => n)} onAdd={(name) => setColors({ series: { ...colors.series, [name]: "#6366f1" } })} />
          </div>
          {rules.colorByValue ? (
            <div className="grid gap-2">
              <SettingSwitch label="Shade each mark by its value" ariaLabel="Colour by value" checked={!!byValue} onCheckedChange={(on) => setColors({ byValue: on ? ["#dbeafe", "#1d4ed8"] : undefined })} />
              {byValue ? (
                <div className="flex flex-wrap items-center gap-1.5">
                  {byValue.map((c, i) => (
                    <ColourInput key={i} value={c} label={`Colour by value stop ${i + 1}`} onChange={(next) => { if (next) setColors({ byValue: byValue.map((x, j) => (j === i ? next : x)) }); }} />
                  ))}
                  <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs"
                    onClick={() => setColors({ byValue: byValue.length === 2 ? [byValue[0]!, "#60a5fa", byValue[1]!] : [byValue[0]!, byValue[2]!] })}>
                    {byValue.length === 2 ? "Add a middle colour" : "Remove the middle colour"}
                  </Button>
                </div>
              ) : null}
            </div>
          ) : null}
        </SettingGroup>
      ) : null}
      {rules.axes ? (
        <SettingGroup title="Axes">
          <AxisEditor title="X" value={axes.x} onChange={(a) => setAxes({ x: a })} canLog={false} rotate />
          <AxisEditor title="Y" value={axes.y} onChange={(a) => setAxes({ y: a })} canLog rotate={false} />
          {rules.secondAxis ? (
            <>
              <AxisEditor title="Second Y" value={axes.y2} onChange={(a) => setAxes({ y2: a })} canLog rotate={false} />
              <fieldset className="grid gap-1.5">
                <legend className="mb-1.5 text-xs font-medium">Values on the right axis</legend>
                {valueNames.map((n) => (
                  <label key={n} className="flex min-w-0 items-center gap-2 text-sm">
                    <input type="checkbox" checked={right.includes(n)} onChange={(e) => setAxes({ rightValues: toggle(right, n, e.target.checked) })} aria-label={`${n} on the right axis`} />
                    <span className="truncate">{n}</span>
                  </label>
                ))}
              </fieldset>
            </>
          ) : null}
        </SettingGroup>
      ) : null}
      {rules.clusters ? (
        <SettingGroup title="Clusters">
          <SettingSwitch label="Colour points by cluster" checked={style.clusters !== undefined} onCheckedChange={(on) => set({ clusters: on ? { k: "auto" } : undefined })} />
          {style.clusters ? (
            <SettingField label="Number of groups">
              {(id) => (
                <Select value={String(style.clusters!.k)} items={CLUSTER_ITEMS} onValueChange={(v) => set({ clusters: { k: !v || v === "auto" ? "auto" : Number(v) } })}>
                  <SelectTrigger id={id} className="w-full" aria-label="Number of clusters"><SelectValue /></SelectTrigger>
                  <SelectContent>{Object.entries(CLUSTER_ITEMS).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
                </Select>
              )}
            </SettingField>
          ) : null}
        </SettingGroup>
      ) : null}
      {rules.todayLine ? (
        <SettingGroup title="Today line">
          <SettingSwitch label="Show a line at today" ariaLabel="Today line" checked={style.todayLine !== false} onCheckedChange={(on) => set({ todayLine: on ? undefined : false })} />
        </SettingGroup>
      ) : null}
      {rules.labels ? (
        <SettingGroup title="Labels">
          <SettingSwitch label="Show values on the chart" checked={style.labels?.show === true} onCheckedChange={(on) => set({ labels: on ? { show: true } : undefined })} />
        </SettingGroup>
      ) : null}
      {children}
      {rules.notes ? (
        <SettingGroup title="Notes">
          {notes.map((a, i) => (
            <div key={i} className="grid grid-cols-[minmax(0,1fr)] gap-3">
              <ItemHeading onRemove={() => set({ annotations: notes.filter((_, j) => j !== i) })} removeLabel={`Remove note ${i + 1}`}>Note {i + 1}</ItemHeading>
              <div className={valueNames.length > 1 ? "grid grid-cols-2 gap-3" : "grid grid-cols-[minmax(0,1fr)]"}>
                <SettingField label="At point">
                  {(id) => <Input id={id} value={a.x} placeholder="e.g. 2024" aria-label={`Note ${i + 1} point`} onChange={(e) => setNote(i, { x: e.target.value })} />}
                </SettingField>
                {valueNames.length > 1 ? (
                  <SettingField label="Series (optional)">
                    {(id) => <Input id={id} value={a.series ?? ""} aria-label={`Note ${i + 1} series`} onChange={(e) => setNote(i, { series: e.target.value || undefined })} />}
                  </SettingField>
                ) : null}
              </div>
              <SettingField label="Text">
                {(id) => <Input id={id} value={a.text} maxLength={MAX_NOTE_LEN} aria-label={`Note ${i + 1} text`} onChange={(e) => setNote(i, { text: e.target.value })} />}
              </SettingField>
            </div>
          ))}
          {notes.length < MAX_ANNOTATIONS ? (
            <div>
              <Button type="button" size="sm" variant="ghost" className="-ml-2 h-7 px-2 text-xs" onClick={() => set({ annotations: [...notes, { x: "", text: "" }] })}>
                <Plus className="size-3" aria-hidden /> Add a note
              </Button>
            </div>
          ) : null}
        </SettingGroup>
      ) : null}
      {rules.tooltip ? (
        <SettingGroup title="Tooltip">
          <fieldset className="grid gap-1.5">
            <legend className="mb-1.5 text-xs font-medium">Show only these values (none checked shows all)</legend>
            {valueNames.map((n) => (
              <label key={n} className="flex min-w-0 items-center gap-2 text-sm">
                <input type="checkbox" checked={tooltip.includes(n)} onChange={(e) => set({ tooltip: { fields: toggle(tooltip, n, e.target.checked) } })} aria-label={`${n} in the tooltip`} />
                <span className="truncate">{n}</span>
              </label>
            ))}
          </fieldset>
        </SettingGroup>
      ) : null}
      {rules.conditional ? (
        <SettingGroup title="Conditional colours">
          <ConditionalEditor value={style.conditional ?? []} onChange={(c) => set({ conditional: c })} columns={seriesNames ?? valueNames} showLabel={false} />
        </SettingGroup>
      ) : null}
    </div>
  );
}

/** Picks a series name to give a colour; the names are the chart's values and any name typed. */
function SeriesAdder({ names, taken, onAdd }: { readonly names: readonly string[]; readonly taken: readonly string[]; readonly onAdd: (name: string) => void }) {
  const [text, setText] = React.useState("");
  const add = () => {
    const name = text.trim();
    if (!name || taken.includes(name)) return;
    onAdd(name);
    setText("");
  };
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <Input className="min-w-0 flex-1" value={text} list="series-names" placeholder="Pick or type a series" aria-label="Series name" onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); add(); } }} />
      <datalist id="series-names">{names.map((n) => <option key={n} value={n} />)}</datalist>
      <Button type="button" size="sm" variant="outline" className="shrink-0" onClick={add}><Plus className="size-3" aria-hidden /> Add</Button>
    </div>
  );
}
