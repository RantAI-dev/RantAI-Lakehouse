"use client";

import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { LINE_KINDS, MAX_REFERENCE_LINES, isComputedLine, type LineKind, type ParamDef, type RefLineDraft } from "@/lib/params";
import { ColourInput } from "./colour-input";

const FIXED = "__fixed__";
const FIRST = "__first__";
const KIND_LABELS: Record<LineKind, string> = {
  value: "A value", average: "Average", median: "Median", min: "Minimum", max: "Maximum", trend: "Linear trend", band: "A band",
};

/**
 * Reference lines of a chart (`BI-11`, `BI-17`): up to five marks on the value
 * axis. A line is at a number or at the value of a number parameter, or it is
 * computed from the rows on screen (average, median, minimum, maximum, a linear
 * trend), or it is a band between two values. A plain number needs no
 * parameter. `valueNames` lets a computed line say which value it describes.
 */
export function ReferenceLinesEditor({
  value, onChange, numberParams, valueNames = [],
}: {
  readonly value: readonly RefLineDraft[];
  readonly onChange: (next: RefLineDraft[]) => void;
  readonly numberParams: readonly ParamDef[];
  readonly valueNames?: readonly string[];
}) {
  const set = (i: number, patch: Partial<RefLineDraft>) => onChange(value.map((l, j) => (j === i ? { ...l, ...patch } : l)));
  const items: Record<string, string> = { [FIXED]: "A number", ...Object.fromEntries(numberParams.map((p) => [p.name, p.label || p.name])) };
  return (
    <div className="grid gap-1.5">
      <Label>Reference lines</Label>
      {value.map((line, i) => {
        const kind: LineKind = line.kind ?? "value";
        const computed = isComputedLine(kind);
        // A number or a parameter for one edge of the line.
        const edge = (which: "from" | "to") => {
          const text = which === "from" ? line.value : line.to ?? "";
          const param = which === "from" ? line.param : line.toParam ?? "";
          const name = which === "from" ? (kind === "band" ? "from" : "value") : "to";
          const patch = (t: string, p: string): Partial<RefLineDraft> => (which === "from" ? { value: t, param: p } : { to: t, toParam: p });
          return (
            <span key={which} className="flex items-center gap-1.5">
              {numberParams.length ? (
                <Select value={param || FIXED} items={items} onValueChange={(v) => set(i, patch("", v && v !== FIXED ? v : ""))}>
                  <SelectTrigger className="w-28 shrink-0" aria-label={`Line ${i + 1} ${which === "to" ? "upper edge follows" : "follows"}`}><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={FIXED}>A number</SelectItem>
                    {numberParams.map((p) => <SelectItem key={p.name} value={p.name}>{p.label || p.name}</SelectItem>)}
                  </SelectContent>
                </Select>
              ) : null}
              {param ? null : (
                <Input className="h-8 w-24" type="number" value={text} placeholder={which === "to" ? "To" : kind === "band" ? "From" : "Value"} aria-label={`Line ${i + 1} ${name}`} onChange={(e) => set(i, patch(e.target.value, ""))} />
              )}
            </span>
          );
        };
        return (
          <div key={i} className="flex flex-wrap items-center gap-1.5">
            <Select value={kind} items={KIND_LABELS} onValueChange={(v) => set(i, { kind: (v ?? "value") as LineKind })}>
              <SelectTrigger className="w-32 shrink-0" aria-label={`Line ${i + 1} kind`}><SelectValue /></SelectTrigger>
              <SelectContent>
                {LINE_KINDS.map((k) => <SelectItem key={k} value={k}>{KIND_LABELS[k]}</SelectItem>)}
              </SelectContent>
            </Select>
            {computed ? null : edge("from")}
            {kind === "band" ? edge("to") : null}
            {computed && valueNames.length > 1 ? (
              <Select value={line.measure || FIRST} items={{ [FIRST]: "First value", ...Object.fromEntries(valueNames.map((n) => [n, n])) }} onValueChange={(v) => set(i, { measure: v && v !== FIRST ? v : undefined })}>
                <SelectTrigger className="w-32 shrink-0" aria-label={`Line ${i + 1} describes`}><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={FIRST}>First value</SelectItem>
                  {valueNames.map((n) => <SelectItem key={n} value={n}>{n}</SelectItem>)}
                </SelectContent>
              </Select>
            ) : null}
            <Input className="h-8 min-w-0 flex-1" value={line.label} maxLength={60} placeholder="Label" aria-label={`Line ${i + 1} label`} onChange={(e) => set(i, { label: e.target.value })} />
            <ColourInput value={line.color} onChange={(c) => set(i, { color: c })} label={`Line ${i + 1} colour`} fallback="#d97706" />
            <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove line ${i + 1}`} onClick={() => onChange(value.filter((_, j) => j !== i))}><X /></Button>
          </div>
        );
      })}
      {value.length < MAX_REFERENCE_LINES ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={() => onChange([...value, { value: "", param: "", label: "" }])}>
            <Plus className="size-3" aria-hidden /> Add a line
          </Button>
        </div>
      ) : null}
    </div>
  );
}
