"use client";

/**
 * The chart builder's numbered sections (phase 2 QA R3, R10): sections 1 to 3
 * are an accordion with a summary line, the last is a fold that opens by itself
 * when the chart being edited uses something inside it.
 */
import * as React from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Sections 1 to 3 as an accordion (phase 2 QA R10): the heading is a button
 * (`aria-expanded`, reachable by Tab); a folded section shows its one-line
 * summary beside the heading. The controls stay mounted while folded, hidden,
 * so what is picked, the preview and a focus request on a missing field keep
 * working; only a closed section's `hidden` panel is taken out of the tab order.
 */
export function AccordionSection({ number, title, summary, open, onOpenChange, panelRef, children }: {
  readonly number: number;
  readonly title: string;
  readonly summary: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly panelRef?: (node: HTMLDivElement | null) => void;
  readonly children: React.ReactNode;
}) {
  const id = React.useId();
  const panel = `${id}-panel`;
  return (
    <section aria-labelledby={id} className="grid gap-3 border-t border-border pt-3">
      <h3 id={id} className="text-sm font-semibold">
        <button
          type="button" aria-expanded={open} aria-controls={panel} onClick={() => onOpenChange(!open)}
          className="flex w-full items-center gap-2 rounded-sm text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span aria-hidden className="grid size-5 shrink-0 place-items-center rounded-full bg-muted text-xs font-medium text-muted-foreground">{number}</span>
          <span className="shrink-0">{title}</span>
          {open ? null : <span className="min-w-0 truncate text-xs font-normal text-muted-foreground" title={summary}>{summary}</span>}
          <ChevronRight aria-hidden className={cn("ml-auto size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
        </button>
      </h3>
      <div id={panel} ref={panelRef} hidden={!open} className="grid gap-3">{children}</div>
    </section>
  );
}

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

/** A section that opens and closes; its controls are not rendered while it is closed. */
export function FoldSection({ number, title, open, onOpenChange, panelRef, children }: {
  readonly panelRef?: (node: HTMLDivElement | null) => void;
  readonly number: number;
  readonly title: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly children: React.ReactNode;
}) {
  const id = React.useId();
  const panel = `${id}-panel`;
  return (
    <section aria-labelledby={id} className="grid gap-3 border-t border-border pt-4">
      <h3 id={id} className="text-sm font-semibold">
        <button
          type="button" aria-expanded={open} aria-controls={panel} onClick={() => onOpenChange(!open)}
          className="flex w-full items-center gap-2 rounded-sm text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span aria-hidden className="grid size-5 place-items-center rounded-full bg-muted text-xs font-medium text-muted-foreground">{number}</span>
          {title}
          <ChevronRight aria-hidden className={cn("ml-auto size-4 text-muted-foreground transition-transform", open && "rotate-90")} />
        </button>
      </h3>
      {open ? <div id={panel} ref={panelRef} className="grid gap-4">{children}</div> : null}
    </section>
  );
}
