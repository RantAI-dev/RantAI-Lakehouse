"use client";

import * as React from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The chart builder's numbered sections (phase 2 QA R3): a short heading and
 * the controls under it. The last one is folded; the builder opens it by
 * itself when the chart being edited uses something inside it.
 */
export function BuilderSection({ number, title, children }: {
  readonly number: number;
  readonly title: string;
  readonly children: React.ReactNode;
}) {
  const id = React.useId();
  return (
    <section aria-labelledby={id} className="grid gap-3 border-t border-border pt-4">
      <h3 id={id} className="flex items-center gap-2 text-sm font-semibold">
        <span aria-hidden className="grid size-5 place-items-center rounded-full bg-muted text-xs font-medium text-muted-foreground">{number}</span>
        {title}
      </h3>
      {children}
    </section>
  );
}

/** A section that opens and closes; its controls are not rendered while it is closed. */
export function FoldSection({ number, title, open, onOpenChange, children }: {
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
      {open ? <div id={panel} className="grid gap-4">{children}</div> : null}
    </section>
  );
}
