"use client";

import * as React from "react";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  BUILTIN_PALETTES, DATE_STYLES, FORMAT_STYLES, MAX_ANNOTATIONS, MAX_AFFIX_LEN, MAX_AXIS_TITLE_LEN, MAX_CONDITIONAL_RULES, MAX_DECIMALS, MAX_NOTE_LEN,
  type Annotation, type AxisStyle, type CompareOp, type CondRule, type CondRuleTest, type DateFormat, type StyleFields, type ValueFormat,
} from "@/lib/chart-style";
import type { FormatRules } from "@/lib/style-draft";
import { ColourInput } from "./colour-input";

/**
 * Section 5 of the chart builder, "Format" (`BI-17` part A, T7): number and date formats, colours,
 * axes, data labels, notes, tooltip fields. Reference lines have their own editor, placed in this
 * section by the builder. A group the chart's kind does not take is not drawn (`formatRules`), so
 * the form never offers what the server would refuse.
 */

const STYLE_LABELS: Record<string, string> = {
  auto: "As it is now", number: "Number", percent: "Percent", currency: "Currency", scientific: "Scientific", compact: "Compact (1.2 K)",
};
const DATE_LABELS: Record<string, string> = {
  auto: "As it is now", short: "15/10/26", medium: "15 Oct 2026", long: "15 October 2026", iso: "2026-10-15", monthYear: "October 2026",
};
const PALETTE_LABELS: Record<string, string> = {
  default: "Default", bright: "Bright", muted: "Muted", "sequential-blue": "Sequential blue", "diverging-red-blue": "Diverging red-blue", company: "Company palette",
};
const COMPARE_LABELS: Record<CompareOp, string> = { lt: "is below", le: "is at most", gt: "is above", ge: "is at least", eq: "equals", ne: "does not equal" };
const RULE_LABELS: Record<CondRuleTest["type"], string> = { compare: "Compare", between: "Between", contains: "Contains", range: "Colour range" };
const ROTATIONS: Record<string, string> = { "": "Automatic", "0": "Flat", "45": "45°", "90": "Upright" };
const ALL_VALUES = "__all__";

function Group({ title, children }: { readonly title: string; readonly children: React.ReactNode }) {
  return (
    <fieldset className="grid gap-2 rounded-lg border border-border p-3">
      <legend className="px-1 text-xs font-medium text-muted-foreground">{title}</legend>
      {children}
    </fieldset>
  );
}

function num(text: string): number | undefined {
  return text.trim() === "" || !Number.isFinite(Number(text)) ? undefined : Number(text);
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
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Select value={style} items={STYLE_LABELS} onValueChange={(v) => set({ style: (v ?? "auto") as ValueFormat["style"] })}>
        <SelectTrigger className="w-40 shrink-0" aria-label={`${label} style`}><SelectValue /></SelectTrigger>
        <SelectContent>{FORMAT_STYLES.map((s) => <SelectItem key={s} value={s}>{STYLE_LABELS[s]}</SelectItem>)}</SelectContent>
      </Select>
      <Input className="h-8 w-20" type="number" min={0} max={MAX_DECIMALS} value={f.decimals ?? ""} placeholder="Decimals" aria-label={`${label} decimals`}
        onChange={(e) => set({ decimals: num(e.target.value) })} />
      {currencyStyle ? (
        <>
          <Input className="h-8 w-20 uppercase" maxLength={3} value={f.currency ?? ""} placeholder="IDR" aria-label={`${label} currency code`}
            onChange={(e) => set({ currency: e.target.value.toUpperCase() || undefined })} />
          <Select value={f.currencyDisplay ?? "symbol"} items={{ symbol: "Symbol", code: "Code" }} onValueChange={(v) => set({ currencyDisplay: v === "code" ? "code" : "symbol" })}>
            <SelectTrigger className="w-24 shrink-0" aria-label={`${label} currency display`}><SelectValue /></SelectTrigger>
            <SelectContent><SelectItem value="symbol">Symbol</SelectItem><SelectItem value="code">Code</SelectItem></SelectContent>
          </Select>
        </>
      ) : null}
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        <Switch size="sm" checked={f.thousands !== false} onCheckedChange={(c) => set({ thousands: c === true ? undefined : false })} aria-label={`${label} thousands separator`} /> 1,000
      </span>
      <Input className="h-8 w-20" maxLength={MAX_AFFIX_LEN} value={f.prefix ?? ""} placeholder="Prefix" aria-label={`${label} prefix`} onChange={(e) => set({ prefix: e.target.value || undefined })} />
      <Input className="h-8 w-20" maxLength={MAX_AFFIX_LEN} value={f.suffix ?? ""} placeholder="Suffix" aria-label={`${label} suffix`} onChange={(e) => set({ suffix: e.target.value || undefined })} />
    </div>
  );
}

function DateFormatEditor({ value, onChange }: { readonly value: DateFormat | undefined; readonly onChange: (next: DateFormat) => void }) {
  const f = value ?? {};
  const set = (patch: Partial<DateFormat>) => onChange({ ...f, ...patch });
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Select value={f.style ?? "auto"} items={DATE_LABELS} onValueChange={(v) => set({ style: (v ?? "auto") as DateFormat["style"] })}>
        <SelectTrigger className="w-44 shrink-0" aria-label="Date style"><SelectValue /></SelectTrigger>
        <SelectContent>{DATE_STYLES.map((s) => <SelectItem key={s} value={s}>{DATE_LABELS[s]}</SelectItem>)}</SelectContent>
      </Select>
      <Select value={f.hour12 === undefined ? "locale" : f.hour12 ? "12" : "24"} items={{ locale: "Time as is", "12": "12-hour", "24": "24-hour" }}
        onValueChange={(v) => set({ hour12: v === "12" ? true : v === "24" ? false : undefined })}>
        <SelectTrigger className="w-28 shrink-0" aria-label="Time format"><SelectValue /></SelectTrigger>
        <SelectContent><SelectItem value="locale">Time as is</SelectItem><SelectItem value="12">12-hour</SelectItem><SelectItem value="24">24-hour</SelectItem></SelectContent>
      </Select>
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        <Switch size="sm" checked={f.abbreviate === true} onCheckedChange={(c) => set({ abbreviate: c === true ? true : undefined })} aria-label="Short month and day names" /> Short names
      </span>
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
    <div className="flex flex-wrap items-center gap-1.5">
      <Input className="h-8 w-36" maxLength={MAX_AXIS_TITLE_LEN} value={a.title ?? ""} placeholder={`${title} title`} aria-label={`${title} axis title`} onChange={(e) => set({ title: e.target.value || undefined })} />
      {canLog ? (
        <>
          <Input className="h-8 w-20" type="number" value={a.min ?? ""} placeholder="Min" aria-label={`${title} axis minimum`} onChange={(e) => set({ min: num(e.target.value) })} />
          <Input className="h-8 w-20" type="number" value={a.max ?? ""} placeholder="Max" aria-label={`${title} axis maximum`} onChange={(e) => set({ max: num(e.target.value) })} />
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            <Switch size="sm" checked={a.log === true} onCheckedChange={(c) => set({ log: c === true ? true : undefined })} aria-label={`${title} axis logarithmic`} /> Log
          </span>
        </>
      ) : null}
      {rotate ? (
        <Select value={a.rotate === undefined ? "" : String(a.rotate)} items={ROTATIONS} onValueChange={(v) => set({ rotate: v === "" || v == null ? undefined : (Number(v) as 0 | 45 | 90) })}>
          <SelectTrigger className="w-28 shrink-0" aria-label={`${title} axis label rotation`}><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(ROTATIONS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        <Switch size="sm" checked={a.show !== false} onCheckedChange={(c) => set({ show: c === true ? undefined : false })} aria-label={`Show the ${title} axis`} /> Show
      </span>
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
    <div className="flex flex-wrap items-center gap-1.5">
      <Select value={rule.column || "__none__"} items={{ __none__: "Column", ...Object.fromEntries(columns.map((c) => [c, c])) }} onValueChange={(v) => onChange({ ...rule, column: v && v !== "__none__" ? v : "" })}>
        <SelectTrigger className="w-32 shrink-0" aria-label={`Rule ${n} column`}><SelectValue /></SelectTrigger>
        <SelectContent>{columns.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
      </Select>
      <Select value={t.type} items={RULE_LABELS} onValueChange={(v) => switchType((v ?? "compare") as CondRuleTest["type"])}>
        <SelectTrigger className="w-32 shrink-0" aria-label={`Rule ${n} kind`}><SelectValue /></SelectTrigger>
        <SelectContent>{Object.entries(RULE_LABELS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
      </Select>
      {t.type === "compare" ? (
        <>
          <Select value={t.op} items={COMPARE_LABELS} onValueChange={(v) => setTest({ ...t, op: (v ?? "lt") as CompareOp })}>
            <SelectTrigger className="w-32 shrink-0" aria-label={`Rule ${n} operator`}><SelectValue /></SelectTrigger>
            <SelectContent>{Object.entries(COMPARE_LABELS).map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
          </Select>
          <Input className="h-8 w-24" type="number" value={Number.isFinite(t.value) ? t.value : ""} aria-label={`Rule ${n} value`} onChange={(e) => setTest({ ...t, value: num(e.target.value) ?? Number.NaN })} />
        </>
      ) : null}
      {t.type === "between" ? (
        <>
          <Input className="h-8 w-20" type="number" value={Number.isFinite(t.from) ? t.from : ""} aria-label={`Rule ${n} from`} onChange={(e) => setTest({ ...t, from: num(e.target.value) ?? Number.NaN })} />
          <Input className="h-8 w-20" type="number" value={Number.isFinite(t.to) ? t.to : ""} aria-label={`Rule ${n} to`} onChange={(e) => setTest({ ...t, to: num(e.target.value) ?? Number.NaN })} />
        </>
      ) : null}
      {t.type === "contains" ? <Input className="h-8 w-32" value={t.text} maxLength={80} aria-label={`Rule ${n} text`} onChange={(e) => setTest({ ...t, text: e.target.value })} /> : null}
      {t.type === "range" ? (
        <>
          {t.colors.map((c, i) => (
            <ColourInput key={i} value={c} label={`Rule ${n} range colour ${i + 1}`} onChange={(next) => { if (next) setTest({ ...t, colors: t.colors.map((x, j) => (j === i ? next : x)) }); }} />
          ))}
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs"
            onClick={() => setTest({ ...t, colors: t.colors.length === 2 ? [t.colors[0]!, "#f59e0b", t.colors[1]!] : [t.colors[0]!, t.colors[2]!] })}>
            {t.colors.length === 2 ? "Add a middle colour" : "Remove the middle colour"}
          </Button>
        </>
      ) : (
        <ColourInput value={rule.color} label={`Rule ${n} colour`} onChange={(c) => onChange({ ...rule, color: c })} fallback="#dc2626" />
      )}
      {t.type !== "range" ? (
        <Select value={rule.target ?? "cell"} items={{ cell: "Colour the cell", text: "Colour the text" }} onValueChange={(v) => onChange({ ...rule, target: v === "text" ? "text" : "cell" })}>
          <SelectTrigger className="w-36 shrink-0" aria-label={`Rule ${n} colours`}><SelectValue /></SelectTrigger>
          <SelectContent><SelectItem value="cell">Colour the cell</SelectItem><SelectItem value="text">Colour the text</SelectItem></SelectContent>
        </Select>
      ) : null}
      <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove rule ${n}`} onClick={onRemove}><X /></Button>
    </div>
  );
}

/** Colours by rule for a table, a pivot or a KPI. */
export function ConditionalEditor({ value, onChange, columns }: {
  readonly value: readonly CondRule[];
  readonly onChange: (next: CondRule[]) => void;
  readonly columns: readonly string[];
}) {
  return (
    <div className="grid gap-1.5">
      <Label>Conditional colours</Label>
      {value.map((rule, i) => (
        <RuleRow key={i} rule={rule} columns={columns} index={i}
          onChange={(next) => onChange(value.map((r, j) => (j === i ? next : r)))} onRemove={() => onChange(value.filter((_, j) => j !== i))} />
      ))}
      {value.length < MAX_CONDITIONAL_RULES ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={() => onChange([...value, { ...NEW_RULE, column: columns.length === 1 ? columns[0]! : "" }])}>
            <Plus className="size-3" aria-hidden /> Add a rule
          </Button>
        </div>
      ) : null}
    </div>
  );
}

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
    <div className="grid gap-3">
      {rules.values ? (
        <Group title="Values">
          {valueNames.length > 1 ? (
            <Select value={formatOf} items={{ [ALL_VALUES]: "All values", ...Object.fromEntries(valueNames.map((n) => [n, n])) }} onValueChange={(v) => setFormatOf(v ?? ALL_VALUES)}>
              <SelectTrigger className="w-48" aria-label="Format of"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL_VALUES}>All values</SelectItem>
                {valueNames.map((n) => <SelectItem key={n} value={n}>{n}</SelectItem>)}
              </SelectContent>
            </Select>
          ) : null}
          <ValueFormatEditor value={current} onChange={setFormat} label="Number format" />
        </Group>
      ) : null}
      {rules.dates ? (
        <Group title="Dates"><DateFormatEditor value={style.dateFormat} onChange={(f) => set({ dateFormat: f })} /></Group>
      ) : null}
      {rules.colors ? (
        <Group title="Colours">
          {/* The palette the person chose is stored as it is, "default" included:
              it is a choice of its own, so a company palette does not take it
              (`PR #199 review O SHOULD-FIX 4`). */}
          <Select value={paletteValue} items={PALETTE_LABELS} onValueChange={(v) => setColors({ palette: v || undefined })}>
            <SelectTrigger className="w-48" aria-label="Palette"><SelectValue /></SelectTrigger>
            <SelectContent>{[...BUILTIN_PALETTES, "company"].map((p) => <SelectItem key={p} value={p}>{PALETTE_LABELS[p]}</SelectItem>)}</SelectContent>
          </Select>
          <div className="grid gap-1">
            <Label className="text-xs text-muted-foreground">Colour of a series</Label>
            {pickedSeries.map(([name, colour]) => (
              <div key={name} className="flex items-center gap-1.5">
                <span className="w-32 truncate text-xs" title={name}>{name}</span>
                <ColourInput value={colour} label={`Colour of ${name}`} onChange={(c) => {
                  const next = { ...colors.series };
                  if (c) next[name] = c; else delete next[name];
                  setColors({ series: next });
                }} />
              </div>
            ))}
            <SeriesAdder names={valueNames} taken={pickedSeries.map(([n]) => n)} onAdd={(name) => setColors({ series: { ...colors.series, [name]: "#6366f1" } })} />
          </div>
          {rules.colorByValue ? (
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="flex items-center gap-1 text-xs">
                <Switch size="sm" checked={!!byValue} onCheckedChange={(c) => setColors({ byValue: c === true ? ["#dbeafe", "#1d4ed8"] : undefined })} aria-label="Colour by value" /> Colour by value
              </span>
              {byValue?.map((c, i) => (
                <ColourInput key={i} value={c} label={`Colour by value stop ${i + 1}`} onChange={(next) => { if (next) setColors({ byValue: byValue.map((x, j) => (j === i ? next : x)) }); }} />
              ))}
              {byValue ? (
                <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs"
                  onClick={() => setColors({ byValue: byValue.length === 2 ? [byValue[0]!, "#60a5fa", byValue[1]!] : [byValue[0]!, byValue[2]!] })}>
                  {byValue.length === 2 ? "Add a middle colour" : "Remove the middle colour"}
                </Button>
              ) : null}
            </div>
          ) : null}
        </Group>
      ) : null}
      {rules.axes ? (
        <Group title="Axes">
          <AxisEditor title="X" value={axes.x} onChange={(a) => setAxes({ x: a })} canLog={false} rotate />
          <AxisEditor title="Y" value={axes.y} onChange={(a) => setAxes({ y: a })} canLog rotate={false} />
          {rules.secondAxis ? (
            <>
              <AxisEditor title="Second Y" value={axes.y2} onChange={(a) => setAxes({ y2: a })} canLog rotate={false} />
              <div className="flex flex-wrap items-center gap-3">
                <span className="text-xs text-muted-foreground">On the right axis</span>
                {valueNames.map((n) => (
                  <label key={n} className="flex items-center gap-1 text-xs">
                    <input type="checkbox" checked={right.includes(n)} onChange={(e) => setAxes({ rightValues: toggle(right, n, e.target.checked) })} aria-label={`${n} on the right axis`} /> {n}
                  </label>
                ))}
              </div>
            </>
          ) : null}
        </Group>
      ) : null}
      {rules.labels ? (
        <Group title="Labels">
          <span className="flex items-center gap-2 text-xs">
            <Switch size="sm" checked={style.labels?.show === true} onCheckedChange={(c) => set({ labels: c === true ? { show: true } : undefined })} aria-label="Show values on the chart" /> Show values on the chart
          </span>
        </Group>
      ) : null}
      {children}
      {rules.notes ? (
        <Group title="Notes">
          {notes.map((a, i) => (
            <div key={i} className="flex flex-wrap items-center gap-1.5">
              <Input className="h-8 w-28" value={a.x} placeholder="Point" aria-label={`Note ${i + 1} point`} onChange={(e) => setNote(i, { x: e.target.value })} />
              {valueNames.length > 1 ? (
                <Input className="h-8 w-28" value={a.series ?? ""} placeholder="Series (optional)" aria-label={`Note ${i + 1} series`} onChange={(e) => setNote(i, { series: e.target.value || undefined })} />
              ) : null}
              <Input className="h-8 min-w-0 flex-1" value={a.text} maxLength={MAX_NOTE_LEN} placeholder="Note" aria-label={`Note ${i + 1} text`} onChange={(e) => setNote(i, { text: e.target.value })} />
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove note ${i + 1}`} onClick={() => set({ annotations: notes.filter((_, j) => j !== i) })}><X /></Button>
            </div>
          ))}
          {notes.length < MAX_ANNOTATIONS ? (
            <div>
              <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={() => set({ annotations: [...notes, { x: "", text: "" }] })}>
                <Plus className="size-3" aria-hidden /> Add a note
              </Button>
            </div>
          ) : null}
        </Group>
      ) : null}
      {rules.tooltip ? (
        <Group title="Tooltip">
          <div className="flex flex-wrap items-center gap-3">
            <span className="text-xs text-muted-foreground">Show only</span>
            {valueNames.map((n) => (
              <label key={n} className="flex items-center gap-1 text-xs">
                <input type="checkbox" checked={tooltip.includes(n)} onChange={(e) => set({ tooltip: { fields: toggle(tooltip, n, e.target.checked) } })} aria-label={`${n} in the tooltip`} /> {n}
              </label>
            ))}
          </div>
        </Group>
      ) : null}
      {rules.conditional ? (
        <Group title="Conditional colours">
          <ConditionalEditor value={style.conditional ?? []} onChange={(c) => set({ conditional: c })} columns={seriesNames ?? valueNames} />
        </Group>
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
    <div className="flex items-center gap-1.5">
      <Input className="h-8 w-40" value={text} list="series-names" placeholder="Series name" aria-label="Series name" onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); add(); } }} />
      <datalist id="series-names">{names.map((n) => <option key={n} value={n} />)}</datalist>
      <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={add}><Plus className="size-3" aria-hidden /> Add colour</Button>
    </div>
  );
}
