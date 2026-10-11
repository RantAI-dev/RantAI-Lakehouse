"use client"

import * as React from "react"

import { columnKindOfType, type ColumnKind } from "@/lib/time-grain"
import { dashboardService } from "@/services"
import type { CalcField } from "@/services/contracts/calc-fields"
import type { MetricSource } from "@/services/contracts/metrics"
import { apiFetch } from "@/services/http"

/** A source a metric or a segment can be defined on. */
export type SourceChoice = { value: string; label: string; source: MetricSource }

/** The columns of one source, with their kinds, and its calculated fields (for a formula's suggestions). */
export type SourceColumns = { columns: { name: string; kind: ColumnKind }[]; calculated: CalcField[] }

/** The marts and SQL sources a definition can be made on. A list that cannot be read is empty. */
export function useSourceChoices(open: boolean): SourceChoice[] {
  const [choices, setChoices] = React.useState<SourceChoice[]>([])
  React.useEffect(() => {
    if (!open) return
    let live = true
    void Promise.all([
      apiFetch("/api/dashboard/fields").then((r) => r.json()).then((j) => (j.marts ?? []) as { name: string }[]).catch(() => []),
      dashboardService.listSqlSources().catch(() => []),
    ]).then(([marts, sources]) => {
      if (!live) return
      setChoices([
        ...marts.map((m) => ({ value: `mart:${m.name}`, label: m.name, source: { mart: m.name } as MetricSource })),
        ...sources.map((s) => ({ value: `sql:${s.id}`, label: `SQL · ${s.title}`, source: { source: s.id } as MetricSource })),
      ])
    })
    return () => { live = false }
  }, [open])
  return choices
}

/** The columns of `source`; empty while loading or when it cannot be read. */
export function useSourceColumns(source: MetricSource | null): SourceColumns {
  const [state, setState] = React.useState<SourceColumns>({ columns: [], calculated: [] })
  const key = source ? (source.mart !== undefined ? `mart=${encodeURIComponent(source.mart)}` : `source=${encodeURIComponent(source.source)}`) : ""
  React.useEffect(() => {
    if (!key) { setState({ columns: [], calculated: [] }); return }
    let live = true
    void apiFetch(`/api/dashboard/fields?${key}`)
      .then((r) => r.json())
      .then((j: { columns?: { name: string; type: string }[]; calculated?: CalcField[] }) => {
        if (!live) return
        setState({ columns: (j.columns ?? []).map((c) => ({ name: c.name, kind: columnKindOfType(c.type) })), calculated: j.calculated ?? [] })
      })
      .catch(() => { if (live) setState({ columns: [], calculated: [] }) })
    return () => { live = false }
  }, [key])
  return state
}
