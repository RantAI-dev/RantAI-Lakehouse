"use client";

import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { DEFAULT_FORECAST_HORIZON, LINE_KINDS, MAX_FORECAST_HORIZON, MAX_REFERENCE_LINES, MIN_FORECAST_HORIZON, isComputedLine, type LineKind, type ParamDef, type RefLineDraft } from "@/lib/params";
import { ColourInput } from "./colour-input";
import { SettingField, SettingGroup } from "./builder-sections";

const FIXED = "__fixed__";
const FIRST = "__first__";
const KIND_LABELS: Record<LineKind, string> = {
  value: "A value", average: "Average", median: "Median", min: "Minimum", max: "Maximum", trend: "Linear trend", band: "A band", forecast: "Forecast",
};

/**
 * Reference lines of a chart (`BI-11`, `BI-17`): up to five marks on the value
 * axis. A line is at a number or at the value of a number parameter, or it is
 * computed from the rows on screen (average, median, minimum, maximum, a linear
 * trend), or it is a band between two values. A plain number needs no
 * parameter. `valueNames` lets a computed line say which value it describes.
 */
export function ReferenceLinesEditor({
  value, onChange, numberParams, valueNames = [], takesForecast = true,
}: {
  readonly value: readonly RefLineDraft[];
  readonly onChange: (next: RefLineDraft[]) => void;
  readonly numberParams: readonly ParamDef[];
  readonly valueNames?: readonly string[];
  /** `BI-17` part B: false for a chart kind that cannot draw a forecast (a horizontal bar chart). */
  readonly takesForecast?: boolean;
}) {
  const set = (i: number, patch: Partial<RefLineDraft>) => onChange(value.map((l, j) => (j === i ? { ...l, ...patch } : l)));
  const items: Record<string, string> = { [FIXED]: "A number", ...Object.fromEntries(numberParams.map((p) => [p.name, p.label || p.name])) };
  // At most one forecast per chart (plan D1): the kind is offered again only to the line that has it.
  const forecastAt = value.findIndex((l) => l.kind === "forecast");
  const offered = (i: number) => LINE_KINDS.filter((k) => k !== "forecast" || (takesForecast && (forecastAt < 0 || forecastAt === i)));
  return (
    // BI-58 review round 1 (SHOULD-FIX 3): a group of the Style tab like the others; each line is a
    // short stack of labelled, full-width fields instead of one row of narrow unlabelled ones.
    <SettingGroup title="Reference lines">
      {value.map((line, i) => {
        const kind: LineKind = line.kind ?? "value";
        const computed = isComputedLine(kind);
        // A number or a parameter for one edge of the line.
        const edge = (which: "from" | "to") => {
          const text = which === "from" ? line.value : line.to ?? "";
          const param = which === "from" ? line.param : line.toParam ?? "";
          const name = which === "from" ? (kind === "band" ? "from" : "value") : "to";
          const word = which === "to" ? "To" : kind === "band" ? "From" : "Value";
          const patch = (t: string, p: string): Partial<RefLineDraft> => (which === "from" ? { value: t, param: p } : { to: t, toParam: p });
          return (
            <div key={which} className={numberParams.length ? "grid grid-cols-2 gap-3" : "grid grid-cols-[minmax(0,1fr)]"}>
              {numberParams.length ? (
                <SettingField label={`${word} from`}>
                  {(id) => (
                    <Select value={param || FIXED} items={items} onValueChange={(v) => set(i, patch("", v && v !== FIXED ? v : ""))}>
                      <SelectTrigger id={id} className="w-full" aria-label={`Line ${i + 1} ${which === "to" ? "upper edge follows" : "follows"}`}><SelectValue /></SelectTrigger>
                      <SelectContent>
                        <SelectItem value={FIXED}>A number</SelectItem>
                        {numberParams.map((p) => <SelectItem key={p.name} value={p.name}>{p.label || p.name}</SelectItem>)}
                      </SelectContent>
                    </Select>
                  )}
                </SettingField>
              ) : null}
              {param ? null : (
                <SettingField label={word}>
                  {(id) => <Input id={id} type="number" value={text} aria-label={`Line ${i + 1} ${name}`} onChange={(e) => set(i, patch(e.target.value, ""))} />}
                </SettingField>
              )}
            </div>
          );
        };
        return (
          <div key={i} className="grid grid-cols-[minmax(0,1fr)] gap-3">
            <div className="flex min-h-6 items-center justify-between gap-2 pt-2">
              <p className="text-sm font-semibold">Line {i + 1}</p>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove line ${i + 1}`} onClick={() => onChange(value.filter((_, j) => j !== i))}><X /></Button>
            </div>
            <SettingField label="Kind">
              {(id) => (
                <Select value={kind} items={KIND_LABELS} onValueChange={(v) => {
                  const next = (v ?? "value") as LineKind;
                  set(i, next === "forecast" && !line.horizon ? { kind: next, horizon: String(DEFAULT_FORECAST_HORIZON) } : { kind: next });
                }}>
                  <SelectTrigger id={id} className="w-full" aria-label={`Line ${i + 1} kind`}><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {offered(i).map((k) => <SelectItem key={k} value={k}>{KIND_LABELS[k]}</SelectItem>)}
                  </SelectContent>
                </Select>
              )}
            </SettingField>
            {computed ? null : edge("from")}
            {kind === "band" ? edge("to") : null}
            {kind === "forecast" ? (
              <SettingField label="Periods ahead">
                {(id) => <Input id={id} type="number" min={MIN_FORECAST_HORIZON} max={MAX_FORECAST_HORIZON} step={1} value={line.horizon ?? ""} aria-label={`Line ${i + 1} horizon`} onChange={(e) => set(i, { horizon: e.target.value })} />}
              </SettingField>
            ) : null}
            {computed && valueNames.length > 1 ? (
              <SettingField label="Of the value">
                {(id) => (
                  <Select value={line.measure || FIRST} items={{ [FIRST]: "First value", ...Object.fromEntries(valueNames.map((n) => [n, n])) }} onValueChange={(v) => set(i, { measure: v && v !== FIRST ? v : undefined })}>
                    <SelectTrigger id={id} className="w-full" aria-label={`Line ${i + 1} describes`}><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value={FIRST}>First value</SelectItem>
                      {valueNames.map((n) => <SelectItem key={n} value={n}>{n}</SelectItem>)}
                    </SelectContent>
                  </Select>
                )}
              </SettingField>
            ) : null}
            <div className="grid grid-cols-[auto_minmax(0,1fr)] items-end gap-3">
              <div className="grid gap-1.5">
                <p className="text-xs font-medium">Colour</p>
                <ColourInput value={line.color} onChange={(c) => set(i, { color: c })} label={`Line ${i + 1} colour`} fallback="#d97706" />
              </div>
              <SettingField label="Label">
                {(id) => <Input id={id} value={line.label} maxLength={60} aria-label={`Line ${i + 1} label`} onChange={(e) => set(i, { label: e.target.value })} />}
              </SettingField>
            </div>
          </div>
        );
      })}
      {value.length < MAX_REFERENCE_LINES ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="-ml-2 h-7 px-2 text-xs" onClick={() => onChange([...value, { value: "", param: "", label: "" }])}>
            <Plus className="size-3" aria-hidden /> Add a line
          </Button>
        </div>
      ) : null}
    </SettingGroup>
  );
}
