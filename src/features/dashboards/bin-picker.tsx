"use client";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  BIN_CHOICES, GRID_LABELS, GRID_SIZES, binChoiceOf, binChoicesFor, binDraftFromChoice, type BinDraft,
} from "@/lib/chart-bins";

/**
 * The control that puts a number grouping in bins (`BI-10`): none, automatic,
 * a set count, or a width of the person's own. Shown only beside a number
 * column; a SQL source offers a width only, because its rows are cut at a
 * fixed cap and the range of an automatic bin would be read from the cut.
 */
export function BinPicker({
  label, value, onChange, sqlSource, disabled, aria,
}: {
  readonly label?: string;
  readonly value: BinDraft | null;
  readonly onChange: (next: BinDraft | null) => void;
  readonly sqlSource: boolean;
  readonly disabled?: boolean;
  readonly aria: string;
}) {
  const choices = binChoicesFor(sqlSource);
  return (
    <div className="mt-1 grid gap-1.5">
      {label ? <Label>{label}</Label> : null}
      <div className="flex items-center gap-1.5">
        <Select value={binChoiceOf(value)} items={BIN_CHOICES} onValueChange={(v) => onChange(binDraftFromChoice(v ?? "none", value))} disabled={disabled}>
          <SelectTrigger className="w-full min-w-0 flex-1" aria-label={aria}><SelectValue /></SelectTrigger>
          <SelectContent>{choices.map((c) => <SelectItem key={c} value={c}>{BIN_CHOICES[c]}</SelectItem>)}</SelectContent>
        </Select>
        {value?.mode === "width" ? (
          <Input
            type="number" min="0" step="any" inputMode="decimal" placeholder="width" value={value.width}
            onChange={(e) => onChange({ ...value, width: e.target.value })} aria-label={`${aria}: width`} className="w-28 shrink-0"
          />
        ) : null}
      </div>
    </div>
  );
}

/** The grid of a point map: one mark per point, or one per cell of 0.01, 0.1, 1 or 10 degrees, or automatic. */
export function GridPicker({
  value, onChange, disabled,
}: {
  readonly value: string;
  readonly onChange: (next: string) => void;
  readonly disabled?: boolean;
}) {
  return (
    <div className="grid gap-1.5">
      <Label>Marks</Label>
      <Select value={value} items={GRID_LABELS} onValueChange={(v) => onChange(v ?? "none")} disabled={disabled}>
        <SelectTrigger className="w-full" aria-label="Marks"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="none">{GRID_LABELS.none}</SelectItem>
          {GRID_SIZES.map((s) => <SelectItem key={s} value={s}>{GRID_LABELS[s]}</SelectItem>)}
        </SelectContent>
      </Select>
    </div>
  );
}
