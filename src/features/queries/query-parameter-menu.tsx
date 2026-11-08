"use client"

import * as React from "react"
import { Plus, SlidersHorizontal, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { retypeDefinition } from "@/lib/query-param-names"
import { isQueryParamType } from "@/lib/query-params"
import type {
  QueryParamDef,
  QueryParamOption,
  QueryParamType,
  QueryParamValue,
} from "@/services/contracts/queries"
import { ParameterControl } from "./query-parameter-control"

/** What each type is called in the menu. */
const TYPE_LABELS: Record<QueryParamType, string> = {
  text: "Text",
  number: "Number",
  date: "Date",
  dateRange: "Date range",
  dropdown: "Dropdown",
  multiSelect: "Multi-select",
}

/**
 * The most options a dropdown or multi-select takes (feature decision 6).
 * The menu only stops adding the 201st; every other rule about a definition
 * is the server's, and its sentence is what the person reads when it
 * refuses.
 */
const MAX_OPTIONS = 200

/**
 * An option's value as the person typed it. The menu keeps option values as
 * text, except that an option which already held a number stays a number
 * while its text is still one: a list mixing the two is refused by the
 * server.
 */
function typedLike(previous: string | number, text: string): string | number {
  if (typeof previous === "number" && text.trim() !== "" && Number.isFinite(Number(text))) {
    return Number(text)
  }
  return text
}

/**
 * The menu that edits a parameter's definition: its type, its default and,
 * for a dropdown or a multi-select, its options. It reports the whole new
 * definition; the studio decides what a changed definition does to the
 * value (a value that no longer fits the type goes).
 */
export function ParameterMenu({
  id,
  def,
  onChange,
}: {
  readonly id: string
  readonly def: QueryParamDef
  readonly onChange: (def: QueryParamDef) => void
}) {
  const isList = def.type === "dropdown" || def.type === "multiSelect"
  const options = def.options ?? []

  // An absent default or option list is left out of the definition, not
  // written as `undefined`: the server reads a missing key as "none".
  function setOptions(next: QueryParamOption[]) {
    const changed: QueryParamDef = { name: def.name, type: def.type }
    if (def.default !== undefined) changed.default = def.default
    if (next.length > 0) changed.options = next
    onChange(changed)
  }

  function setDefault(value: QueryParamValue | undefined) {
    const changed: QueryParamDef = { name: def.name, type: def.type }
    if (value !== undefined) changed.default = value
    if (def.options) changed.options = def.options
    onChange(changed)
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={`Edit definition of ${def.name}`}
        >
          <SlidersHorizontal className="size-3" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 gap-3">
        <div className="space-y-1">
          <Label htmlFor={`${id}-type`}>Type</Label>
          <Select
            value={def.type}
            items={TYPE_LABELS}
            onValueChange={(next) => {
              if (isQueryParamType(next)) onChange(retypeDefinition(def, next))
            }}
          >
            <SelectTrigger id={`${id}-type`} size="sm" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(Object.keys(TYPE_LABELS) as QueryParamType[]).map((type) => (
                <SelectItem key={type} value={type}>
                  {TYPE_LABELS[type]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          {/* A range has two inputs, so its label names the group instead. */}
          <Label
            id={`${id}-default-label`}
            htmlFor={def.type === "dateRange" ? undefined : `${id}-default`}
          >
            Default
          </Label>
          <div className="flex items-center gap-2">
            <ParameterControl
              id={`${id}-default`}
              def={def}
              value={def.default}
              onChange={setDefault}
            />
            {def.default !== undefined ? (
              <Button type="button" variant="ghost" size="xs" onClick={() => setDefault(undefined)}>
                Clear default
              </Button>
            ) : null}
          </div>
        </div>
        {isList ? (
          <div className="space-y-2">
            <p className="text-xs font-medium text-muted-foreground">Options</p>
            {options.map((option, index) => (
              <div key={index} className="flex items-center gap-1">
                <Input
                  aria-label={`Option ${index + 1} label`}
                  placeholder="Label"
                  className="h-7 text-xs"
                  value={option.label}
                  onChange={(event) =>
                    setOptions(
                      options.map((open, at) =>
                        at === index ? { ...open, label: event.target.value } : open
                      )
                    )
                  }
                />
                <Input
                  aria-label={`Option ${index + 1} value`}
                  placeholder="Value"
                  className="h-7 text-xs"
                  value={String(option.value)}
                  onChange={(event) =>
                    setOptions(
                      options.map((open, at) =>
                        at === index
                          ? { ...open, value: typedLike(open.value, event.target.value) }
                          : open
                      )
                    )
                  }
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove option ${index + 1}`}
                  onClick={() => setOptions(options.filter((_, at) => at !== index))}
                >
                  <X className="size-3" aria-hidden />
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="outline"
              size="xs"
              disabled={options.length >= MAX_OPTIONS}
              title={options.length >= MAX_OPTIONS ? `A list takes at most ${MAX_OPTIONS} options.` : undefined}
              onClick={() => setOptions([...options, { value: "", label: "" }])}
            >
              <Plus className="size-3" aria-hidden />
              Add option
            </Button>
          </div>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}
