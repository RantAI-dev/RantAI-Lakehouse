"use client"

import * as React from "react"

import { Label } from "@/components/ui/label"
import type { QueryParamDef, QueryParamValue } from "@/services/contracts/queries"
import { ParameterControl } from "./query-parameter-control"
import { ParameterMenu } from "./query-parameter-menu"

/**
 * One field for each `{{name}}` in the open tab's SQL (`QS-5`), in the order
 * the names first appear. Each field matches its parameter's type and has a
 * small menu that edits the definition. A field starts at its definition's
 * default when the tab holds no value for it.
 *
 * The fields hold no state of their own beyond what a half-typed input
 * needs: the studio keeps the definitions and the values in the tab.
 */
export function QueryParameterFields({
  definitions,
  values,
  invalid,
  onValueChange,
  onDefinitionChange,
}: {
  /** The definitions of the names the SQL uses, in order. */
  readonly definitions: readonly QueryParamDef[]
  /** The tab's values by name. A name without one is not given. */
  readonly values: Readonly<Record<string, QueryParamValue>>
  /**
   * The names the server's last refusal blamed. Their fields are marked
   * invalid; the sentence itself is shown where the run's error is.
   */
  readonly invalid?: ReadonlySet<string>
  /** `undefined` means the field holds no value now. */
  readonly onValueChange: (name: string, value: QueryParamValue | undefined) => void
  readonly onDefinitionChange: (def: QueryParamDef) => void
}) {
  if (definitions.length === 0) return null
  return (
    <div role="group" aria-label="Query parameters" className="flex flex-wrap items-start gap-x-4 gap-y-2">
      {definitions.map((def) => (
        // The type is part of the key: a field whose type changed starts
        // over, with none of the old control's half-typed text.
        <ParameterField
          key={`${def.name}:${def.type}`}
          def={def}
          value={Object.hasOwn(values, def.name) ? values[def.name] : def.default}
          invalid={invalid?.has(def.name) ?? false}
          onValueChange={(value) => onValueChange(def.name, value)}
          onDefinitionChange={onDefinitionChange}
        />
      ))}
    </div>
  )
}

function ParameterField({
  def,
  value,
  invalid,
  onValueChange,
  onDefinitionChange,
}: {
  readonly def: QueryParamDef
  readonly value: QueryParamValue | undefined
  readonly invalid: boolean
  readonly onValueChange: (value: QueryParamValue | undefined) => void
  readonly onDefinitionChange: (def: QueryParamDef) => void
}) {
  const id = React.useId()
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1">
        {def.type === "dateRange" ? (
          <span className="text-xs font-medium">{def.name}</span>
        ) : (
          <Label htmlFor={id} className="text-xs">
            {def.name}
          </Label>
        )}
        <ParameterMenu id={id} def={def} onChange={onDefinitionChange} />
      </div>
      <ParameterControl id={id} def={def} value={value} invalid={invalid} onChange={onValueChange} />
    </div>
  )
}
