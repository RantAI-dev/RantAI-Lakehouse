"use client"

import * as React from "react"
import { ChevronDown, ChevronRight, Lock, RotateCcw, X } from "lucide-react"

import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { quoteIdent } from "@/lib/asset-query"
import type { CompletionTable } from "@/lib/sql-completion"
import { DATA_LAYER_LABEL, type DataLayer } from "@/lib/status"
import type { useQueryStudio } from "./use-query-studio"

/** Why a table with no SQL name cannot be inserted. */
const NO_SQL_NAME =
  "Its SQL name is not known here: the Trino name is on the asset's detail page."

/** The layers in the order the console lists them everywhere. */
const LAYERS = Object.keys(DATA_LAYER_LABEL) as DataLayer[]

/** A table's reference as SQL, each dotted part quoted when it is not a plain identifier. */
function quotedReference(reference: string): string {
  return reference.split(".").map(quoteIdent).join(".")
}

/**
 * The catalog beside the editor: layers, the tables under them, and a
 * table's columns once it is expanded. A click puts the name in the editor
 * at the cursor.
 *
 * It is a panel next to the editor, never in place of it: whatever the
 * catalog answers, the editor stays usable.
 */
export function SchemaBrowser({
  studio,
  onClose,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
  readonly onClose: () => void
}) {
  const { assetsState, tables, columns, loadColumns, insertAtCursor } = studio
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set())

  const toggle = (id: string) => {
    const opening = !expanded.has(id)
    setExpanded((previous) => {
      const next = new Set(previous)
      if (opening) next.add(id)
      else next.delete(id)
      return next
    })
    if (opening) loadColumns(id)
  }

  return (
    <aside
      aria-label="Schema browser"
      className="flex max-h-[32rem] min-w-0 flex-col rounded-md border border-border bg-background"
    >
      <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
        <h2 className="text-sm font-semibold">Schema</h2>
        <Button
          size="icon-sm"
          variant="ghost"
          onClick={onClose}
          aria-label="Close schema browser"
        >
          <X className="size-4" aria-hidden />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2 text-sm">
        {assetsState.status === "loading" ? (
          <LoadingSkeleton rows={4} />
        ) : assetsState.status === "error" ? (
          assetsState.error.code === "permission_denied" ? (
            // The shared permission state does not say which permission is
            // missing; here the person needs to know it is the catalog.
            <div className="flex flex-col items-center gap-2 px-3 py-6 text-center" role="status">
              <Lock className="size-5 text-muted-foreground" aria-hidden />
              <p className="font-medium">The schema browser needs catalog access</p>
              <p className="text-muted-foreground">
                Your role cannot read the catalog. The editor still runs SQL; ask a workspace
                administrator for catalog access to browse tables here.
              </p>
            </div>
          ) : (
            <ErrorState error={assetsState.error} className="px-3 py-6" />
          )
        ) : tables.length === 0 ? (
          <p className="px-2 py-4 text-muted-foreground">The catalog shows no tables.</p>
        ) : (
          LAYERS.map((layer) => {
            const inLayer = tables.filter((table) => table.layer === layer)
            if (inLayer.length === 0) return null
            return (
              <section key={layer} className="mb-3">
                <h3 className="px-2 py-1 text-xs font-medium text-muted-foreground">
                  {DATA_LAYER_LABEL[layer]}
                </h3>
                <ul>
                  {inLayer.map((table) => (
                    <TableRow
                      key={table.id}
                      table={table}
                      open={expanded.has(table.id)}
                      state={columns[table.id]}
                      onToggle={() => toggle(table.id)}
                      onInsert={insertAtCursor}
                      onRetry={() => loadColumns(table.id, { retry: true })}
                    />
                  ))}
                </ul>
              </section>
            )
          })
        )}
      </div>
    </aside>
  )
}

function TableRow({
  table,
  open,
  state,
  onToggle,
  onInsert,
  onRetry,
}: {
  readonly table: CompletionTable
  readonly open: boolean
  readonly state: ReturnType<typeof useQueryStudio>["columns"][string] | undefined
  readonly onToggle: () => void
  readonly onInsert: (text: string) => void
  readonly onRetry: () => void
}) {
  const insertable = table.reference !== ""
  return (
    <li>
      <div className="flex items-center gap-1">
        <Button
          size="icon-sm"
          variant="ghost"
          onClick={onToggle}
          aria-expanded={open}
          aria-label={`Columns of ${table.label}`}
        >
          {open ? (
            <ChevronDown className="size-4" aria-hidden />
          ) : (
            <ChevronRight className="size-4" aria-hidden />
          )}
        </Button>
        {/* A registry slug is not a table name: inserting it would write a
            query that fails, so the row is shown but does nothing. */}
        <button
          type="button"
          className="min-w-0 flex-1 truncate rounded px-1 py-1 text-left hover:bg-muted aria-disabled:cursor-not-allowed aria-disabled:opacity-60 aria-disabled:hover:bg-transparent"
          aria-disabled={!insertable}
          title={insertable ? `Insert ${table.reference}` : NO_SQL_NAME}
          aria-label={insertable ? `Insert ${table.reference}` : table.label}
          onClick={insertable ? () => onInsert(quotedReference(table.reference)) : undefined}
        >
          <span>{table.label}</span>
          {insertable ? (
            <span className="ml-2 font-mono text-xs text-muted-foreground">
              {table.reference}
            </span>
          ) : null}
        </button>
      </div>
      {open ? (
        <div className="ml-8 border-l border-border pl-2">
          {!state || state.status === "loading" ? (
            <p className="py-1 text-xs text-muted-foreground">Loading columns…</p>
          ) : state.status === "failed" ? (
            <div className="py-1 text-xs">
              <p className="text-muted-foreground">
                Columns could not be loaded{state.reason ? `: ${state.reason}` : "."}
              </p>
              <Button
                size="xs"
                variant="outline"
                onClick={onRetry}
                aria-label={`Retry columns of ${table.label}`}
              >
                <RotateCcw className="size-3" aria-hidden />
                Retry
              </Button>
            </div>
          ) : (
            <ul>
              {state.columns.map((column) => (
                <li key={column.name}>
                  <button
                    type="button"
                    className="flex w-full items-baseline justify-between gap-2 rounded px-1 py-0.5 text-left hover:bg-muted"
                    aria-label={`Insert column ${quoteIdent(column.name)}`}
                    onClick={() => onInsert(quoteIdent(column.name))}
                  >
                    <span className="truncate font-mono text-xs">{column.name}</span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {column.dataType}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </li>
  )
}
