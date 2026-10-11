"use client";

import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { GRAIN_LABELS, isGrain, type Grain } from "@/lib/time-grain";

const OWN = "__own__";

/**
 * The dashboard's grain control (BI-9), in the filter row. It regroups every
 * chart that has a day-to-year grain at once; "Each chart's own" puts them
 * back. Keeping the choice is the filter bar's one "Save as default" (BI-9
 * review fix R4).
 */
export function GrainSwitch({
  choices, value, onChange,
}: {
  readonly choices: readonly Grain[];
  /** The grain shown; "" is each chart's own. */
  readonly value: Grain | "";
  readonly onChange: (next: Grain | "") => void;
}) {
  const items: Record<string, string> = { [OWN]: "Each chart's own", ...Object.fromEntries(choices.map((g) => [g, GRAIN_LABELS[g]])) };
  return (
    <div className="flex items-center gap-2">
      <Label className="text-xs text-muted-foreground">Group by</Label>
      <Select value={value || OWN} items={items} onValueChange={(v) => onChange(v && v !== OWN && isGrain(v) ? v : "")}>
        <SelectTrigger size="sm" className="w-40" aria-label="Group dates by"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value={OWN}>Each chart&apos;s own</SelectItem>
          {choices.map((g) => <SelectItem key={g} value={g}>{GRAIN_LABELS[g]}</SelectItem>)}
        </SelectContent>
      </Select>
    </div>
  );
}
