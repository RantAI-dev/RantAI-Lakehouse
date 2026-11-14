"use client";

import * as React from "react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { GRAIN_LABELS, type Grain } from "@/lib/time-grain";
import { ChipPanelLabel, ChipTrigger, ChoiceRow, chipShell } from "./filters/chip";

const OWN_LABEL = "Each chart's own";

/**
 * The dashboard's grain control (BI-9), in the filter row: a chip ("Group by
 * Month ⌄", `BI-61` T3) whose panel lists the choices. It regroups every
 * chart that has a day-to-year grain at once; "Each chart's own" puts them
 * back. Keeping the choice is the filter row's one "Save as default" (BI-9
 * review fix R4). The chip is tinted while the grain differs from the saved one.
 */
export function GrainSwitch({
  choices, value, onChange, changed = false,
}: {
  readonly choices: readonly Grain[];
  /** The grain shown; "" is each chart's own. */
  readonly value: Grain | "";
  readonly onChange: (next: Grain | "") => void;
  /** The grain shown differs from the dashboard's saved one. */
  readonly changed?: boolean;
}) {
  const [open, setOpen] = React.useState(false);
  const pick = (next: Grain | "") => { onChange(next); setOpen(false); };
  const tone = changed ? "changed" : "default";
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <span className={chipShell(tone)}>
        <PopoverTrigger asChild>
          <ChipTrigger label="Group by" value={value ? GRAIN_LABELS[value] : OWN_LABEL} tone={tone} aria-label="Group dates by" />
        </PopoverTrigger>
      </span>
      <PopoverContent align="start" collisionPadding={16} className="w-56 gap-2 p-3">
        <ChipPanelLabel>Group dates by</ChipPanelLabel>
        <div role="radiogroup" aria-label="Group dates by" className="-mx-1 grid gap-0.5">
          <ChoiceRow on={value === ""} label={OWN_LABEL} onClick={() => pick("")} />
          {choices.map((g) => <ChoiceRow key={g} on={value === g} label={GRAIN_LABELS[g]} onClick={() => pick(g)} />)}
        </div>
      </PopoverContent>
    </Popover>
  );
}
