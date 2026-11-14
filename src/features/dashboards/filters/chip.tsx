"use client";

/**
 * The borderless chip every filter, parameter and the date grouping of a
 * dashboard is drawn as (`BI-61` T3, layout A): its name, its value and a
 * chevron, opening a small panel. A chip whose value differs from the saved
 * one is tinted, so a changed view shows at a glance; a refused one is
 * tinted red. Badges and chips carry no border (DESIGN.md §3).
 */
import * as React from "react";
import { Check, ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";
import { EASE, FOCUS } from "../dashboard-header";

export type ChipTone = "default" | "changed" | "refused";

const TONE: Record<ChipTone, string> = {
  default: "bg-muted/70 text-foreground hover:bg-muted dark:bg-input/30 dark:hover:bg-input/50",
  changed: "bg-primary/12 text-primary hover:bg-primary/18 dark:text-sky-200",
  refused: "bg-destructive/10 text-destructive hover:bg-destructive/15",
};

/** The chip's container: the trigger, and a remove button beside it when there is one. */
export function chipShell(tone: ChipTone): string {
  // Micro-interaction audit 2026-11-14: `:active` holds on the ancestors of what is pressed, so the
  // whole chip gives a little under the pointer whether its value or its × is pressed.
  return cn("inline-flex h-8 max-w-80 shrink-0 items-center rounded-full active:scale-[0.97]", TONE[tone], EASE);
}

/**
 * The part of the chip that opens its panel. Spread onto a Radix
 * `PopoverTrigger asChild`, which passes the ref and the open state.
 */
export function ChipTrigger({
  label, value, tone = "default", className, ...props
}: React.ComponentProps<"button"> & {
  /** The quiet name before the value ("Top berapa"); absent when the value says it all. */
  readonly label?: string;
  readonly value: string;
  readonly tone?: ChipTone;
}) {
  return (
    <button
      type="button"
      {...props}
      className={cn(
        "group/chip inline-flex h-8 min-w-0 shrink-0 items-center gap-1.5 rounded-full px-3 text-sm font-medium whitespace-nowrap",
        "data-[state=open]:ring-2 data-[state=open]:ring-ring/40",
        EASE, FOCUS, className,
      )}
    >
      {label ? <span className={cn("shrink-0 font-normal", tone === "default" ? "text-muted-foreground" : "opacity-80")}>{label}</span> : null}
      <span className="min-w-0 truncate tabular-nums">{value}</span>
      <ChevronDown className={cn("size-3.5 shrink-0 opacity-60 group-data-[state=open]/chip:rotate-180", EASE)} aria-hidden />
    </button>
  );
}

/** The small heading of a chip's panel: a visible label, never only a placeholder (DESIGN.md §3). */
export function ChipPanelLabel({ children, htmlFor }: { readonly children: React.ReactNode; readonly htmlFor?: string }) {
  return htmlFor
    ? <label htmlFor={htmlFor} className="text-xs font-medium text-muted-foreground">{children}</label>
    : <p className="text-xs font-medium text-muted-foreground">{children}</p>;
}

/**
 * One choice of a list, as a full-width row with a tick: the list parameters
 * and the date grouping. A row, not a select: a select's own popup would sit
 * outside the chip's panel and close it on the first click.
 */
export function ChoiceRow({
  on, label, onClick,
}: { readonly on: boolean; readonly label: string; readonly onClick: () => void }) {
  return (
    <button type="button" role="radio" aria-checked={on} onClick={onClick}
      className={cn("flex h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-sm hover:bg-muted", on && "font-medium", EASE, FOCUS)}>
      {/* Micro-interaction audit 2026-11-14: the tick grows into place rather than blinking on. */}
      <Check className={cn("size-4 shrink-0 text-primary", on ? "scale-100 opacity-100" : "scale-75 opacity-0", EASE)} aria-hidden />
      <span className="truncate">{label}</span>
    </button>
  );
}
