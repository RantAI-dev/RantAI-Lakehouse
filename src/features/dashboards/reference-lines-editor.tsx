"use client";

import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { MAX_REFERENCE_LINES, type ParamDef, type RefLineDraft } from "@/lib/params";

const FIXED = "__fixed__";

/**
 * Reference lines of a chart (`BI-11`): up to five horizontal lines on the
 * value axis, each at a number or at the value of a number parameter, with an
 * optional label. A plain number needs no parameter.
 */
export function ReferenceLinesEditor({
  value, onChange, numberParams,
}: {
  readonly value: readonly RefLineDraft[];
  readonly onChange: (next: RefLineDraft[]) => void;
  readonly numberParams: readonly ParamDef[];
}) {
  const set = (i: number, patch: Partial<RefLineDraft>) => onChange(value.map((l, j) => (j === i ? { ...l, ...patch } : l)));
  const items: Record<string, string> = { [FIXED]: "A number", ...Object.fromEntries(numberParams.map((p) => [p.name, p.label || p.name])) };
  return (
    <div className="grid gap-1.5">
      <Label>Reference lines</Label>
      {value.map((line, i) => (
        <div key={i} className="flex items-center gap-1.5">
          {numberParams.length ? (
            <Select value={line.param || FIXED} items={items} onValueChange={(v) => set(i, { param: v && v !== FIXED ? v : "" })}>
              <SelectTrigger className="w-32 shrink-0" aria-label={`Line ${i + 1} follows`}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={FIXED}>A number</SelectItem>
                {numberParams.map((p) => <SelectItem key={p.name} value={p.name}>{p.label || p.name}</SelectItem>)}
              </SelectContent>
            </Select>
          ) : null}
          {line.param ? null : (
            <Input className="h-8 w-28" type="number" value={line.value} placeholder="Value" aria-label={`Line ${i + 1} value`} onChange={(e) => set(i, { value: e.target.value })} />
          )}
          <Input className="h-8 min-w-0 flex-1" value={line.label} maxLength={60} placeholder="Label" aria-label={`Line ${i + 1} label`} onChange={(e) => set(i, { label: e.target.value })} />
          <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove line ${i + 1}`} onClick={() => onChange(value.filter((_, j) => j !== i))}><X /></Button>
        </div>
      ))}
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
