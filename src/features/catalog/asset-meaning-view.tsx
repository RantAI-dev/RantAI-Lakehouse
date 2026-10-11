"use client"

/**
 * The meaning of a table and of its columns on the Schema tab: the table's
 * card above the Columns card, and the Meaning view that the Columns card
 * switches to. The pure rules (which keys have an entry, how entries split,
 * how a save body is built) are in `asset-meaning.ts`.
 *
 * This step shows and sorts the meanings. Nothing here writes yet: the role
 * select and the confirm button render with no save wired, and the edit
 * dialog is not built. The next step wires the saves through the state's
 * `save`.
 */
import { Check } from "lucide-react"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { formatRelativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SemanticEntry } from "@/services/contracts/semantic"
import type { SchemaRow } from "./asset-columns"
import { ROLE_OPTIONS, roleLabel, type MeaningState } from "./asset-meaning"

/** Who wrote an entry, in words; the writer's id is a uuid and stays off the page. */
function ProvenanceLine({ entry }: { entry: SemanticEntry }) {
  const when = formatRelativeTime(entry.updatedAt)
  const who =
    entry.status === "draft"
      ? entry.model
        ? `Drafted by ${entry.model}`
        : "Drafted by the assistant"
      : "Confirmed by a person"
  return <p className="text-xs text-muted-foreground">{`${who} · ${when}`}</p>
}

const TABLE_DESCRIPTION = "What this table is, in plain words. The assistant reads it when it answers a question."

/**
 * The table's own meaning, above the Columns card. Every state the page can
 * be in is said here, so the card is never blank: loading, a failure with
 * its retry, a deployment that does not serve meanings, and a table with no
 * meaning yet.
 */
export function TableMeaningCard({ state }: { state: MeaningState }) {
  if (state.kind === "none") return null
  if (state.kind === "loading") {
    return (
      <SectionCard size="sm" title="Meaning" description={TABLE_DESCRIPTION}>
        <Skeleton className="h-3.5 w-3/4" />
      </SectionCard>
    )
  }
  if (state.kind === "error") {
    return (
      <SectionCard size="sm" title="Meaning" description={TABLE_DESCRIPTION}>
        <p className="flex flex-wrap items-center gap-2 text-sm">
          {state.message}
          <Button size="sm" variant="ghost" onClick={state.retry}>
            Retry
          </Button>
        </p>
      </SectionCard>
    )
  }
  if (state.kind === "unsupported") {
    return (
      <SectionCard size="sm" title="Meaning" description={TABLE_DESCRIPTION}>
        <p className="text-sm text-muted-foreground">{state.reason}</p>
      </SectionCard>
    )
  }
  const { table, canEdit } = state
  const draft = table?.status === "draft"
  return (
    <SectionCard
      size="sm"
      title="Meaning"
      description={TABLE_DESCRIPTION}
      action={
        canEdit ? (
          <span className="flex items-center gap-1.5">
            {draft ? (
              <Button size="sm" variant="outline">
                Confirm
              </Button>
            ) : null}
            <Button size="sm" variant="ghost">
              {table ? "Edit" : "Add a meaning"}
            </Button>
          </span>
        ) : null
      }
    >
      {table ? (
        <div className="flex flex-col gap-2">
          <p className="text-sm">{table.description}</p>
          {table.synonyms.length > 0 ? (
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-xs text-muted-foreground">Also called</span>
              {table.synonyms.map((s) => (
                <Pill key={s} tone="neutral">
                  {s}
                </Pill>
              ))}
            </div>
          ) : null}
          <div className="flex flex-wrap items-center gap-2">
            {draft ? <Pill tone="warning">AI draft</Pill> : null}
            <ProvenanceLine entry={table} />
          </div>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">No meaning is written for this table yet.</p>
      )}
    </SectionCard>
  )
}

// ── The Meaning view ──────────────────────────────────────────────────────
//
// Header and rows share one grid string, as the Columns card does: fixed
// tracks in `rem`, flexible ones `minmax(0, …fr)`, so a row truncates and
// never widens the list. Cells shed by container query: the synonyms go
// first, below `@2xl`; the status goes second, below `@3xl`. The column name,
// the role, the meaning and the confirm control never shed. Every breakpoint
// lists the cells that are shown, in order, so the header and the rows agree.
const MEANING_ROW = "grid items-center gap-x-3 px-3"
const MEANING_TRACKS = [
  "grid-cols-[minmax(0,1fr)_minmax(0,1fr)_1.75rem]",
  "@lg:grid-cols-[minmax(0,1fr)_9rem_minmax(0,2fr)_1.75rem]",
  "@2xl:grid-cols-[minmax(0,1fr)_9rem_minmax(0,2fr)_minmax(0,1fr)_1.75rem]",
  "@3xl:grid-cols-[minmax(0,1fr)_9rem_minmax(0,2fr)_minmax(0,1fr)_5rem_1.75rem]",
].join(" ")

/** One row of the Meaning view: a column of the table, with its entry when it has one. */
function MeaningRow({ row, entry, canEdit }: { row: SchemaRow; entry: SemanticEntry | undefined; canEdit: boolean }) {
  const name = row.column.name
  const draft = entry?.status === "draft"
  const role = entry?.role ?? null
  return (
    <div role="row" className={cn(MEANING_ROW, MEANING_TRACKS, "min-h-9 border-b py-1.5 last:border-b-0")}>
      <div role="cell" className="min-w-0">
        <span className="block truncate font-mono text-xs font-medium" title={name}>
          {name}
        </span>
      </div>
      <div role="cell" className="min-w-0">
        {canEdit ? (
          <Select value={role ?? ""} disabled={!entry}>
            <SelectTrigger size="sm" aria-label={`Role of ${name}`} className="w-full">
              <SelectValue>{roleLabel(role)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="">Not set</SelectItem>
              {ROLE_OPTIONS.map((o) => (
                <SelectItem key={o.value} value={o.value}>
                  {o.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <span className="block truncate text-xs">{roleLabel(role)}</span>
        )}
      </div>
      <div role="cell" className="min-w-0">
        {entry ? (
          canEdit ? (
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Edit the meaning of ${name}`}
              title={entry.description}
              className="h-auto w-full min-w-0 justify-start px-1 py-0.5 text-left text-xs font-normal"
            >
              <span className="block min-w-0 truncate">{entry.description}</span>
            </Button>
          ) : (
            <span className="block truncate text-xs" title={entry.description}>
              {entry.description}
            </span>
          )
        ) : canEdit ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-auto justify-start px-1 py-0.5 text-xs font-normal text-muted-foreground"
          >
            Add a meaning
          </Button>
        ) : (
          <span className="block truncate text-xs text-muted-foreground">No meaning written</span>
        )}
      </div>
      <div role="cell" className="hidden min-w-0 @2xl:block">
        <span className="block truncate text-xs text-muted-foreground" title={entry?.synonyms.join(", ")}>
          {entry?.synonyms.join(", ") ?? ""}
        </span>
      </div>
      <div role="cell" className="hidden @3xl:block">
        {entry ? <Pill tone={draft ? "warning" : "neutral"}>{entry.status}</Pill> : null}
      </div>
      <div role="cell" className="flex justify-end">
        {canEdit && draft ? (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Confirm the draft of ${name}`}
            title="Confirm this draft as it stands"
          >
            <Check className="size-4" aria-hidden />
          </Button>
        ) : null}
      </div>
    </div>
  )
}

/** The header of the Meaning view, with the same grid string as its rows. */
function MeaningHeader() {
  return (
    <div
      role="row"
      className={cn(MEANING_ROW, MEANING_TRACKS, "border-b bg-muted/30 py-2 text-xs font-medium text-muted-foreground")}
    >
      <div role="columnheader">Column</div>
      <div role="columnheader">Role</div>
      <div role="columnheader">Meaning</div>
      <div role="columnheader" className="hidden @2xl:block">
        Also called
      </div>
      <div role="columnheader" className="hidden @3xl:block">
        Status
      </div>
      <div role="columnheader">
        <span className="sr-only">Confirm</span>
      </div>
    </div>
  )
}

/**
 * The Meaning view: one line per listed column, with its role, its meaning,
 * its synonyms and its status. A column without an entry says so in place.
 */
export function MeaningView({
  rows,
  byColumn,
  canEdit,
}: {
  rows: SchemaRow[]
  byColumn: Map<string, SemanticEntry>
  canEdit: boolean
}) {
  return (
    <div className="@container overflow-hidden rounded-lg border border-border">
      <div role="table" aria-label="Column meanings">
        <div role="rowgroup">
          <MeaningHeader />
        </div>
        <div role="rowgroup">
          {rows.map((row) => (
            <MeaningRow key={row.column.name} row={row} entry={byColumn.get(row.column.name)} canEdit={canEdit} />
          ))}
        </div>
      </div>
    </div>
  )
}
