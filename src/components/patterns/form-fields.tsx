"use client"

import * as React from "react"

import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

/**
 * The parts of a calm dialog form (owner QA 2026-11-16, the alert and
 * subscriptions dialogs; `DESIGN.md` §3 "Form dan panel pengaturan"):
 *
 * - a group is a small heading and space, never a box inside the dialog;
 * - every field has a visible label above a control that takes the full width
 *   of its column, so one form never mixes narrow and wide controls;
 * - under a field there is at most one short line (its detail goes in a
 *   `HelpHint`).
 *
 * Every container is a flex column with `min-w-0`: a CSS grid with an implicit
 * `auto` column grows to its widest unbreakable line (a `truncate` row is
 * `nowrap`), and that is what pushed the alert dialog's fields past its edge.
 *
 * The chart builder's `SettingGroup` / `SettingField` (`BI-61` branch, not yet
 * merged here) follow the same rules; fold one into the other when both land.
 */
export function FormGroup({
  title,
  headingId,
  children,
  className,
}: {
  /** Left out for a form that is one group; the dialog's title names it. */
  readonly title?: React.ReactNode
  /** The heading's id, for a control the heading names (a group of one field). */
  readonly headingId?: string
  readonly children: React.ReactNode
  readonly className?: string
}) {
  const ownId = React.useId()
  const id = headingId ?? ownId
  return (
    <section aria-labelledby={title ? id : undefined} className={cn("flex min-w-0 flex-col gap-3", className)}>
      {title ? (
        <h3 id={id} className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          {title}
        </h3>
      ) : null}
      {children}
    </section>
  )
}

/** One field: its visible label, the control (given `id`), and at most one short line under it. */
export function FormField({
  id,
  label,
  hint,
  children,
  className,
}: {
  readonly id: string
  readonly label: React.ReactNode
  readonly hint?: React.ReactNode
  readonly children: React.ReactNode
  readonly className?: string
}) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-1.5", className)}>
      <Label htmlFor={id} className="gap-1">
        {label}
      </Label>
      {children}
      {hint ? <FormHint>{hint}</FormHint> : null}
    </div>
  )
}

/** The one short line under a field or a switch, with room for a `HelpHint` at its end. */
export function FormHint({ children, className }: { readonly children: React.ReactNode; readonly className?: string }) {
  return <p className={cn("flex min-w-0 items-center gap-1 text-xs text-muted-foreground", className)}>{children}</p>
}

/**
 * A switch on a row of its own: the label (and its one line) on the left, the
 * switch on the right. The label names the switch and pressing it toggles.
 */
export function SwitchRow({
  label,
  hint,
  checked,
  onCheckedChange,
  disabled,
}: {
  readonly label: string
  readonly hint?: React.ReactNode
  readonly checked: boolean
  readonly onCheckedChange: (on: boolean) => void
  readonly disabled?: boolean
}) {
  const id = React.useId()
  return (
    <div className="flex min-h-8 min-w-0 items-center justify-between gap-4">
      <div className="flex min-w-0 flex-col gap-1">
        <Label htmlFor={id} className={cn("font-normal", disabled && "text-muted-foreground")}>
          {label}
        </Label>
        {hint ? <FormHint>{hint}</FormHint> : null}
      </div>
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={(on) => onCheckedChange(on === true)} />
    </div>
  )
}
