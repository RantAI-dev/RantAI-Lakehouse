"use client";

import { X } from "lucide-react";
import { Button } from "@/components/ui/button";

/**
 * A colour that may be unset. Unset reads "Colour" (the chart's own colour
 * applies); set shows a colour well and a way back. Always `#rrggbb`, the only
 * form the server accepts.
 */
export function ColourInput({ value, onChange, label, fallback = "#6366f1" }: {
  readonly value: string | undefined;
  readonly onChange: (next: string | undefined) => void;
  readonly label: string;
  /** The colour a first click picks. */
  readonly fallback?: string;
}) {
  if (!value) {
    return <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" aria-label={label} onClick={() => onChange(fallback)}>Colour</Button>;
  }
  return (
    <span className="flex items-center gap-0.5">
      <input type="color" value={value} aria-label={label} onChange={(e) => onChange(e.target.value)} className="h-7 w-8 cursor-pointer rounded border border-border bg-transparent p-0.5" />
      <Button type="button" size="icon-xs" variant="ghost" aria-label={`Clear ${label.toLowerCase()}`} onClick={() => onChange(undefined)}><X /></Button>
    </span>
  );
}
