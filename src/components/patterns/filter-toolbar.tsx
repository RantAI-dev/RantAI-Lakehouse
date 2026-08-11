"use client"

import { Search } from "lucide-react"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"

/** Horizontal toolbar row that holds search and filter controls. */
export function FilterToolbar({
  children,
  className,
}: {
  children: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      {children}
    </div>
  )
}

/** Debounce-free controlled search input with icon. */
export function SearchField({
  value,
  onChange,
  placeholder = "Search...",
  className,
}: {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
}) {
  return (
    <div className={cn("relative w-full max-w-xs", className)}>
      <Search
        className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        aria-hidden
      />
      <Input
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="h-9 pl-9"
        aria-label={placeholder}
      />
    </div>
  )
}

export type FilterOption = { value: string; label: string }

/**
 * Labeled select for toolbar filters. Always includes an "All" option with
 * value "all" so filters can be cleared consistently.
 */
export function FilterSelect({
  value,
  onChange,
  options,
  allLabel,
  ariaLabel,
  className,
}: {
  value: string
  onChange: (value: string) => void
  options: FilterOption[]
  allLabel: string
  ariaLabel: string
  className?: string
}) {
  return (
    <Select
      value={value}
      onValueChange={(next) => {
        if (next != null) onChange(next)
      }}
    >
      <SelectTrigger
        className={cn("h-9 w-auto min-w-36 gap-1", className)}
        aria-label={ariaLabel}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="all">{allLabel}</SelectItem>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
