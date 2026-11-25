"use client";

/**
 * Small pieces of the chart builder: focusing a missing field, the column
 * labels of its pickers, and the segmented control of its header. The numbered
 * accordion sections that lived here (phase 2 QA R3, R10) became the
 * inspector's tabs (`BI-58` T4).
 */
import * as React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
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

/*
 * BI-58 review round 1 (SHOULD-FIX 3, owner's QA 2026-11-14; DESIGN.md §3, "Form dan panel
 * pengaturan rapi dan lega"): the inspector's settings are groups under a small heading,
 * separated by space, never boxes inside the panel; every field has a visible label above it
 * and spans the panel's width; a switch reads as a sentence that says what it does.
 */

/** A group of settings under a small heading. */
export function SettingGroup({ title, children }: { readonly title: string; readonly children: React.ReactNode }) {
  const id = React.useId();
  return (
    <section aria-labelledby={id} className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <h3 id={id} className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>
      {children}
    </section>
  );
}

/** One field: its visible label above the control, which receives the id to be labelled by. */
export function SettingField({ label, hint, children, className }: {
  readonly label: React.ReactNode;
  readonly hint?: React.ReactNode;
  readonly children: (id: string) => React.ReactNode;
  readonly className?: string;
}) {
  const id = React.useId();
  return (
    <div className={cn("grid min-w-0 content-start gap-1.5", className)}>
      <Label htmlFor={id}>{label}</Label>
      {children(id)}
      {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
    </div>
  );
}

/** A switch on a row of its own, with the sentence it turns on as its label. */
export function SettingSwitch({ label, checked, onCheckedChange, ariaLabel, disabled }: {
  readonly label: React.ReactNode;
  readonly checked: boolean;
  readonly onCheckedChange: (on: boolean) => void;
  /** The accessible name when it should differ from the label (kept stable for the tests that find it). */
  readonly ariaLabel?: string;
  readonly disabled?: boolean;
}) {
  const id = React.useId();
  return (
    <div className="flex min-h-8 items-center justify-between gap-3">
      <Label htmlFor={id} className="text-sm font-normal">{label}</Label>
      <Switch id={id} size="sm" checked={checked} disabled={disabled} onCheckedChange={(c) => onCheckedChange(c === true)} aria-label={ariaLabel} />
    </div>
  );
}

/**
 * BI-58 review round 1, SHOULD-FIX 5 (owner): how wide the chart's tile is on its dashboard, shown
 * as two small drawings of a dashboard row (the owner did not read "Half width / Full width"): two
 * tiles side by side, or one long tile, with this chart's tile in the primary colour. A radio group:
 * Tab reaches the chosen option, the arrow keys move between them. The values are the spec's `span`
 * (1 or 2), unchanged.
 */
export function SpanPicker({ value, onChange }: { readonly value: 1 | 2; readonly onChange: (next: 1 | 2) => void }) {
  const options = [{ span: 1 as const, label: "Half row" }, { span: 2 as const, label: "Full row" }];
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const move = (from: number, step: number) => {
    const next = (from + step + options.length) % options.length;
    onChange(options[next]!.span);
    refs.current[next]?.focus();
  };
  return (
    <div role="radiogroup" aria-label="Width on the dashboard" className="grid grid-cols-2 gap-3">
      {options.map((o, i) => {
        const on = o.span === value;
        return (
          <button
            key={o.span} ref={(n) => { refs.current[i] = n; }} type="button" role="radio" aria-checked={on} tabIndex={on ? 0 : -1}
            onClick={() => onChange(o.span)}
            onKeyDown={(e) => {
              if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); move(i, 1); }
              if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); move(i, -1); }
            }}
            className={cn(
              "group/span grid gap-2 rounded-lg p-2 text-left outline-none",
              "transition-[background-color,box-shadow] duration-200 ease-[cubic-bezier(0.2,0,0,1)] focus-visible:ring-3 focus-visible:ring-ring/50",
              on ? "bg-primary/8 ring-2 ring-primary dark:ring-sky-400" : "bg-muted/50 hover:bg-muted",
            )}
          >
            <span aria-hidden className="flex h-10 gap-1 rounded-md bg-card p-1.5">
              {o.span === 1 ? (
                <>
                  <span className={cn("flex-1 rounded-sm transition-colors duration-200", on ? "bg-primary" : "bg-primary/45 group-hover/span:bg-primary/60")} />
                  <span className="flex-1 rounded-sm bg-muted-foreground/20" />
                </>
              ) : (
                <span className={cn("flex-1 rounded-sm transition-colors duration-200", on ? "bg-primary" : "bg-primary/45 group-hover/span:bg-primary/60")} />
              )}
            </span>
            <span className={cn("text-xs font-medium", on ? "text-foreground" : "text-muted-foreground")}>{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
