"use client";

/**
 * Small pieces of the chart builder: focusing a missing field, the column
 * labels of its pickers, and the segmented control of its header. The numbered
 * accordion sections that lived here (phase 2 QA R3, R10) became the
 * inspector's tabs (`BI-58` T4).
 */
import * as React from "react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { ColumnRef } from "@/lib/chart-shape";

/**
 * Moves focus to the field a section is missing: the first select that shows
 * its placeholder or text input that is empty, else the first control. Called
 * once the section is open, since a hidden control cannot take focus.
 */
export function focusFirstEmpty(panel: HTMLElement | null): boolean {
  if (!panel) return false;
  const controls = [...panel.querySelectorAll<HTMLElement>("input:not([type=hidden]):not([disabled]), textarea:not([disabled]), button:not([disabled]), [role=combobox]:not([disabled])")];
  const empty = controls.find((c) => c.hasAttribute("data-placeholder") || ((c instanceof HTMLInputElement || c instanceof HTMLTextAreaElement) && c.value === ""));
  const target = empty ?? controls[0];
  target?.focus();
  return !!target;
}

/** The `items` of a column select: every name with its label, so the closed trigger reads like the list (phase 2 QA R14). */
export function columnItems(names: readonly string[], label: (name: string) => React.ReactNode, current?: string): Record<string, React.ReactNode> {
  const all = current && !names.includes(current) ? [...names, current] : names;
  return Object.fromEntries(all.map((n) => [n, label(n)]));
}

/**
 * A column as the pickers show it (phase 2 QA R14): its readable text, truncated with an
 * ellipsis inside whatever holds it, and a small tag after it (the joined table, "fx",
 * "metric"). The full reference is the hover title, so a long name is never lost, only folded.
 */
export function ColumnLabel({ reference, title }: { readonly reference: ColumnRef; readonly title?: string }) {
  const tagClass = reference.tag === "metric" ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground";
  return (
    <span className="flex min-w-0 items-center gap-1.5" title={title ?? reference.full}>
      <span className="min-w-0 truncate">{reference.text}</span>
      {reference.tag ? <span className={cn("max-w-24 shrink-0 truncate rounded px-1 text-[10px] font-medium", tagClass)}>{reference.tag}</span> : null}
    </span>
  );
}

/**
 * A row of mutually exclusive choices, each a real button reachable by Tab
 * (`BI-58`, from layout A): the width of a chart, Half or Full.
 */
export function Segmented<T extends string>({ value, options, onChange, label, className }: {
  readonly value: T;
  readonly options: readonly { value: T; label: string }[];
  readonly onChange: (next: T) => void;
  /** Read by screen readers; what the choice is about. */
  readonly label: string;
  readonly className?: string;
}) {
  return (
    <div role="group" aria-label={label} className={cn("inline-flex rounded-lg bg-muted p-0.5", className)}>
      {options.map((o) => {
        const on = o.value === value;
        return (
          <button
            key={o.value} type="button" aria-pressed={on} onClick={() => onChange(o.value)}
            className={cn(
              "inline-flex h-7 items-center justify-center rounded-md px-2.5 text-xs font-medium whitespace-nowrap outline-none",
              "transition-[background-color,color,box-shadow] duration-200 ease-[cubic-bezier(0.2,0,0,1)] focus-visible:ring-3 focus-visible:ring-ring/50",
              on ? "bg-background text-foreground shadow-sm dark:bg-input/40" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * A whole number the person types, such as the chart's limit (`BI-58` T6, QA
 * 2026-11-14): the box can be emptied and typed into, and each value in range
 * is applied as it is typed. On leaving the box, an empty or unreadable value
 * goes back to the last one applied, and one out of range is brought to the
 * nearest end. (It used to snap to the default on every keystroke, so the
 * first digit could not be deleted.)
 */
export function WholeNumberInput({ value, min, max, onCommit, ...props }: Omit<React.ComponentProps<typeof Input>, "value" | "onChange" | "min" | "max" | "type"> & {
  readonly value: number;
  readonly min: number;
  readonly max: number;
  readonly onCommit: (next: number) => void;
}) {
  const [text, setText] = React.useState(String(value));
  const [editing, setEditing] = React.useState(false);
  const shown = editing ? text : String(value);
  return (
    <Input
      {...props} type="number" inputMode="numeric" min={min} max={max} value={shown}
      onFocus={() => { setText(String(value)); setEditing(true); }}
      onChange={(e) => {
        const raw = e.target.value;
        setText(raw);
        setEditing(true);
        const n = Number(raw);
        if (raw.trim() !== "" && Number.isInteger(n) && n >= min && n <= max) onCommit(n);
      }}
      onBlur={() => {
        const n = Number(text);
        if (text.trim() !== "" && Number.isFinite(n)) onCommit(Math.max(min, Math.min(max, Math.round(n))));
        setEditing(false);
      }}
    />
  );
}
