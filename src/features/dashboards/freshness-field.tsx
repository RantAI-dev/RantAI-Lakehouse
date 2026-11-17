"use client";

import * as React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { CACHE_MINUTES, MODE_LABELS, draftProblem, isMode, type RuleDraft } from "@/lib/result-cache";

/**
 * The "Freshness" choice of a dashboard, a chart or the install (`BI-4` part B,
 * feature decision 2): follow the default, always fresh, or remember for N
 * minutes. The range is said before a save; the server checks it again.
 */
export function FreshnessField({
  id, label = "Freshness", draft, onChange, disabled, help,
}: {
  readonly id: string;
  readonly label?: string;
  readonly draft: RuleDraft;
  readonly onChange: (d: RuleDraft) => void;
  readonly disabled?: boolean;
  readonly help?: string;
}) {
  const problem = draftProblem(draft);
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={`${id}-mode`}>{label}</Label>
      <div className="flex items-center gap-2">
        <Select value={draft.mode} items={MODE_LABELS} disabled={disabled} onValueChange={(v) => { if (isMode(v)) onChange({ ...draft, mode: v }); }}>
          <SelectTrigger id={`${id}-mode`} className="w-full" aria-label={label}><SelectValue /></SelectTrigger>
          <SelectContent>
            {Object.entries(MODE_LABELS).map(([v, text]) => <SelectItem key={v} value={v}>{text}</SelectItem>)}
          </SelectContent>
        </Select>
        {draft.mode === "minutes" ? (
          <Input
            type="number" inputMode="numeric" className="w-24" min={CACHE_MINUTES.min} max={CACHE_MINUTES.max}
            aria-label={`${label}: minutes`} aria-invalid={problem ? true : undefined}
            value={draft.minutes} disabled={disabled}
            onChange={(e) => onChange({ ...draft, minutes: e.target.value })}
          />
        ) : null}
      </div>
      {problem ? <p role="alert" className="text-xs text-destructive">{problem}</p> : help ? <p className="text-xs text-muted-foreground">{help}</p> : null}
    </div>
  );
}
