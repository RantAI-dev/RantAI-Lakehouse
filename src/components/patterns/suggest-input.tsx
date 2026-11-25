"use client"

import * as React from "react"
import {
  Combobox, ComboboxContent, ComboboxInput, ComboboxItem, ComboboxList,
} from "@/components/ui/combobox"

/** A suggestion: the text it puts in the field, and an optional name shown beside it. */
export type Suggestion = { readonly value: string; readonly label?: string }

/**
 * A text field that suggests names from a list and still takes any name typed
 * (`UX-7`). It replaces the browser's `<datalist>`, which is drawn by the
 * browser and not by the console (owner QA 2026-11-20). The text is the value:
 * choosing a suggestion writes it into the field, typing writes what is typed,
 * so a name the list has not caught up with is accepted as before.
 *
 * Filtering is done here (`filter={null}` on the Base UI root) because a
 * suggestion matches on its value and on its label, and so that the popup can
 * stay closed when nothing matches instead of showing an empty box.
 */
export function SuggestInput({
  id, value, onValueChange, suggestions, className, disabled, title, ...input
}: {
  readonly id?: string
  readonly value: string
  readonly onValueChange: (next: string) => void
  readonly suggestions: readonly Suggestion[]
  readonly className?: string
  readonly disabled?: boolean
  readonly title?: string
} & Omit<React.ComponentProps<"input">, "value" | "onChange" | "id" | "className" | "disabled" | "title" | "list">) {
  const [open, setOpen] = React.useState(false)
  const query = value.trim().toLowerCase()
  // A field already holding one of the names (a saved or default value) lists
  // them all when opened, so the other choices are not hidden behind it.
  const exact = suggestions.some((s) => s.value.toLowerCase() === query)
  const shown = React.useMemo(
    () => suggestions.filter((s) => !query || exact || s.value.toLowerCase().includes(query) || (s.label ?? "").toLowerCase().includes(query)),
    [suggestions, query, exact],
  )
  const labels = React.useMemo(() => new Map(suggestions.map((s) => [s.value, s.label])), [suggestions])
  return (
    <Combobox
      items={shown.map((s) => s.value)}
      inputValue={value}
      onInputValueChange={(next: string) => onValueChange(next)}
      filter={null}
      open={open && shown.length > 0}
      onOpenChange={setOpen}
    >
      <ComboboxInput
        id={id}
        className={className}
        title={title}
        disabled={disabled}
        showTrigger={suggestions.length > 0}
        autoComplete="off"
        {...input}
      />
      <ComboboxContent>
        <ComboboxList>
          {(name: string) => (
            <ComboboxItem key={name} value={name}>
              <span className="min-w-0 flex-1 truncate">{name}</span>
              {labels.get(name) ? (
                <span className="shrink-0 text-xs text-muted-foreground">{labels.get(name)}</span>
              ) : null}
            </ComboboxItem>
          )}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  )
}
