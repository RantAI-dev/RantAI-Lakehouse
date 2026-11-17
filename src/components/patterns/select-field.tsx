"use client"

import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

/** One choice: the value a form stores and the words a person reads. */
export type SelectOption = { value: string; label: string; disabled?: boolean }

/** A named run of choices, the console's form of `<optgroup>`. */
export type SelectOptionGroup = { label: string; options: SelectOption[] }

export type SelectFieldProps = {
  /** Goes on the trigger, so an existing `<Label htmlFor>` still names it. */
  id?: string
  value: string
  onValueChange: (value: string) => void
  options: readonly (SelectOption | SelectOptionGroup)[]
  /** Shown while `value` matches no option. */
  placeholder?: string
  disabled?: boolean
  /** On the trigger: a site keeps its width by passing `w-full`. */
  className?: string
  size?: "sm" | "default"
  "aria-label"?: string
  "aria-invalid"?: boolean
  /** The id of a sentence about this choice, e.g. why it is refused. */
  "aria-describedby"?: string
}

/**
 * The console's dropdown: the design system's `Select` behind a data-only
 * API (`WEB-9` D2), so a site names its choices and cannot grow its own
 * class string.
 *
 * The trigger always shows an option's label, never its raw value. A value
 * that matches no option shows the placeholder; a site that keeps an option
 * for a stored value its list lacks (`WEB-8` D8) adds that option itself.
 */
export function SelectField({
  id,
  value,
  onValueChange,
  options,
  placeholder,
  disabled,
  className,
  size,
  "aria-label": ariaLabel,
  "aria-invalid": ariaInvalid,
  "aria-describedby": ariaDescribedBy,
}: SelectFieldProps) {
  const entries = options.flatMap((option) => ("options" in option ? option.options : [option]))
  const items = entries.map((option) => ({
    value: encoded(option.value),
    label: option.label,
  }))
  // `WEB-9` D2: Base UI treats `""` as "nothing selected", so a real empty
  // choice (`None`, `All`) is held as a private sentinel and translated back
  // on the way out. No site sees it.
  const selected = entries.some((option) => option.value === value) ? encoded(value) : null

  return (
    <Select
      id={id}
      value={selected}
      items={items}
      onValueChange={(next) => onValueChange(decode(next))}
      disabled={disabled}
    >
      <SelectTrigger
        className={className}
        size={size}
        aria-label={ariaLabel}
        aria-invalid={ariaInvalid}
        aria-describedby={ariaDescribedBy}
      >
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) =>
          "options" in option ? (
            <SelectGroup key={option.label}>
              <SelectLabel>{option.label}</SelectLabel>
              {option.options.map((entry) => (
                <OptionItem key={entry.value} option={entry} />
              ))}
            </SelectGroup>
          ) : (
            <OptionItem key={option.value} option={option} />
          )
        )}
      </SelectContent>
    </Select>
  )
}

/** The one value Base UI cannot represent for a real choice. */
const EMPTY_SENTINEL = "__rantai_select_field_empty__"

function encoded(value: string): string {
  return value === "" ? EMPTY_SENTINEL : value
}

function decode(value: string | null): string {
  return value === null || value === EMPTY_SENTINEL ? "" : value
}

function OptionItem({ option }: { option: SelectOption }) {
  return (
    <SelectItem value={encoded(option.value)} disabled={option.disabled}>
      {option.label}
    </SelectItem>
  )
}
