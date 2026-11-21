"use client"

import * as React from "react"
import { ChevronRight, Lock, RotateCcw, Search, Table2 } from "lucide-react"

import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { quoteIdent, NO_READABLE_TABLE } from "@/lib/asset-query"
import type { CompletionTable } from "@/lib/sql-completion"
import { DATA_LAYER_LABEL, type DataLayer } from "@/lib/status"
import { cn } from "@/lib/utils"
import type { useQueryStudio } from "./use-query-studio"

/** Why a table with no SQL name cannot be inserted. */
const NO_SQL_NAME =
  "Its SQL name is not known here: the Trino name is on the asset's detail page."

/**
 * Why a Bronze table is not inserted on a ClickHouse tab: its only SQL name
 * is Trino's (QS-4 QA of 2026-11-05).
 */
const BRONZE_NEEDS_TRINO =
  "Bronze tables are read with Trino: switch this tab to Trino to insert it."

/**
 * What an entry says while its Iceberg table read has failed: the read
 * failed, not the table, so the no-table sentence stays out until a read
 * answers "not found" (#448 review SHOULD-FIX).
 */
const TABLE_PROOF_FAILED = "The table could not be read."

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
 * It is the Schema tab of the studio's side panel (`UX-9`), never in place
 * of the editor: whatever the catalog answers, the editor stays usable. The
 * panel's toggle shows and hides it, so it has no close button of its own.
 */
export function SchemaBrowser({
  studio,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
}) {
  const { assetsState, tables, columns, engine, loadColumns, insertAtCursor, proofs } = studio
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set())
  // QS-11 (LH-QS-1-07), D10: a local, contains-match filter over the labels
  // and SQL references the browser already holds. It is not stored: a search
  // is a moment's narrowing, not a preference.
  const [search, setSearch] = React.useState("")
  const needle = search.trim().toLowerCase()
  const matched = needle
    ? tables.filter(
        (table) =>
          table.label.toLowerCase().includes(needle) ||
          table.reference.toLowerCase().includes(needle)
      )
    : tables

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
    <div className="flex min-w-0 flex-col">
      {assetsState.status === "success" && tables.length > 0 ? (
        <div className="relative px-2 pt-2">
          <Search
            className="pointer-events-none absolute top-1/2 left-4.5 mt-1 size-3.5 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <input
            type="search"
            aria-label="Search tables"
            placeholder="Search tables"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="h-8 w-full rounded-md bg-muted/70 pr-2 pl-8 text-[13px] outline-none placeholder:text-muted-foreground focus-visible:ring-3 focus-visible:ring-ring/50 dark:bg-input/30"
          />
        </div>
      ) : null}
      <div className="p-2 text-[13px]">
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
        ) : matched.length === 0 ? (
          // QS-11 (LH-QS-1-07), D10: say the search found nothing rather than
          // drawing an empty browser that looks like a broken catalog.
          <p className="px-2 py-4 text-muted-foreground">
            No table matches &quot;{search}&quot;.
          </p>
        ) : (
          LAYERS.map((layer) => {
            const inLayer = matched.filter((table) => table.layer === layer)
            if (inLayer.length === 0) return null
            return (
              <section key={layer} className="mb-2">
                <h3 className="px-2 py-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
                  {DATA_LAYER_LABEL[layer]}
                </h3>
                <ul>
                  {inLayer.map((table) => (
                    <TableRow
                      key={table.id}
                      table={table}
                      engine={engine}
                      open={expanded.has(table.id)}
                      state={columns[table.id]}
                      proof={proofs[table.id]}
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
    </div>
  )
}

function TableRow({
  table,
  engine,
  open,
  state,
  proof,
  onToggle,
  onInsert,
  onRetry,
}: {
  readonly table: CompletionTable
  readonly engine: ReturnType<typeof useQueryStudio>["engine"]
  readonly open: boolean
  readonly state: ReturnType<typeof useQueryStudio>["columns"][string] | undefined
  readonly proof: ReturnType<typeof useQueryStudio>["proofs"][string] | undefined
  readonly onToggle: () => void
  readonly onInsert: (text: string) => void
  readonly onRetry: () => void
}) {
  const insertable = table.reference !== ""
  // Why the table cannot be inserted, in the order a person meets them: a
  // ClickHouse tab reads a Bronze table with Trino, a Bronze table whose
  // Iceberg table is not there has no table to read at all, and one whose
  // read failed says so until the read answers (#448 review SHOULD-FIX).
  const missingReason =
    table.trinoOnly && engine !== "trino"
      ? BRONZE_NEEDS_TRINO
      : proof?.status === "missing"
        ? NO_READABLE_TABLE
        : proof?.status === "failed"
          ? TABLE_PROOF_FAILED
          : NO_SQL_NAME
  // LH-QS-4-08: on a Trino tab a Bronze table's columns follow
  // its row. A row that is not insertable (proving, failed) has columns that
  // are not either, and a row whose Iceberg table is not there has none to
  // show. A ClickHouse tab reads a Bronze table with Trino anyway, so its
  // columns stay as they were.
  const columnsFollowRow = table.trinoOnly === true && engine === "trino"
  const columnsLocked = columnsFollowRow && !insertable
  const noTable = columnsFollowRow && proof?.status === "missing"
  return (
    <li>
      <div className="flex items-center gap-0.5 rounded-md hover:bg-muted/60">
        <Button
          size="icon-xs"
          variant="ghost"
          onClick={onToggle}
          aria-expanded={open}
          aria-label={`Columns of ${table.label}`}
        >
          <ChevronRight
            className={cn(
              "size-3.5 text-muted-foreground transition-transform duration-150 ease-ui",
              open && "rotate-90"
            )}
            aria-hidden
          />
        </Button>
        {/* A registry slug is not a table name: inserting it would write a
            query that fails, so the row is shown but does nothing. */}
        <button
          type="button"
          className="flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded px-1 text-left outline-none focus-visible:ring-3 focus-visible:ring-ring/50 aria-disabled:cursor-not-allowed aria-disabled:opacity-60"
          aria-disabled={!insertable}
          title={insertable ? `Insert ${table.reference}` : missingReason}
          aria-label={insertable ? `Insert ${table.reference}` : table.label}
          onClick={
            insertable
              ? // A Trino name is formed and quoted already; splitting it on
                // its dots would quote the quotes.
                () => onInsert(table.trinoOnly ? table.reference : quotedReference(table.reference))
              : undefined
          }
        >
          <Table2 className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <span className="truncate font-medium">{table.label}</span>
          {insertable ? (
            <span className="ml-auto truncate pl-1 font-mono text-[11px] text-muted-foreground">
              {table.reference}
            </span>
          ) : null}
        </button>
      </div>
      {open ? (
        <div className="mb-1 ml-[18px] border-l border-border/70 pl-2">
          {noTable ? (
            <p className="py-1 text-xs text-muted-foreground">{NO_READABLE_TABLE}</p>
          ) : !state || state.status === "loading" ? (
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
                    className="flex h-6 w-full items-center justify-between gap-2 rounded px-1.5 text-left outline-none transition-colors duration-150 ease-ui hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent"
                    aria-label={`Insert column ${quoteIdent(column.name)}`}
                    disabled={columnsLocked}
                    title={columnsLocked ? missingReason : undefined}
                    onClick={() => onInsert(quoteIdent(column.name))}
                  >
                    <span className="truncate font-mono text-xs">{column.name}</span>
                    <span className="truncate text-[11px] text-muted-foreground">
                      {column.dataType}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          {proof?.status === "failed" ? (
            // A failed read is not an answer about the table: it says so and
            // is read again here, on the next open or on request.
            <div className="py-1 text-xs">
              <p className="text-muted-foreground">{TABLE_PROOF_FAILED}</p>
              <Button
                size="xs"
                variant="outline"
                onClick={onRetry}
                aria-label={`Retry reading the table of ${table.label}`}
              >
                <RotateCcw className="size-3" aria-hidden />
                Retry
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
    </li>
  )
}
