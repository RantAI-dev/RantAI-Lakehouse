"use client"

import type * as React from "react"

import type { SourceChoice } from "./use-source-columns"

/**
 * A source as the pickers of the metric and segment editors name it (phase 2 QA R20): its readable
 * name, with a small "SQL" tag for a saved SQL source. The stored value (`mart:...`, `sql:<id>`)
 * never reaches the screen.
 */
export function SourceChoiceLabel({ choice }: { readonly choice: Pick<SourceChoice, "label" | "tag"> }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="truncate">{choice.label}</span>
      {choice.tag ? <span className="shrink-0 rounded bg-muted px-1 text-[10px] font-medium text-muted-foreground">{choice.tag}</span> : null}
    </span>
  )
}

/** The `items` of a `Select` over `choices`, so the closed trigger shows the same label as the list. */
export function sourceChoiceItems(choices: readonly SourceChoice[]): Record<string, React.ReactNode> {
  return Object.fromEntries(choices.map((c) => [c.value, <SourceChoiceLabel key={c.value} choice={c} />]))
}
