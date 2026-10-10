"use client"

/**
 * The Schema tab's "Columns" card, shaped as a column explorer: one compact
 * line per column (type glyph, name and marks, type, a bar of its most
 * frequent values, null share, distinct count, range), and pressing a line
 * opens that column in place, underneath, with every listed value and the
 * column's facts. Plan: `docs/superpowers/plans/2026-10-05-schema-tab-column-explorer.md`.
 *
 * Only what the profile states is drawn. Shares are of the rows profiled;
 * the route lists at most five values and only where the count is exact, so
 * the part of a bar that no value takes is the bare track, never a figure.
 *
 * # Why a grid and not a `<table>`
 *
 * The shared `Table` wraps its table in an `overflow-x-auto` scroller, and
 * a table of seven columns cannot narrow below the sum of its cells, so at
 * phone width it scrolled sideways. The list here is `div`s with the ARIA
 * table roles instead: every row is a CSS grid whose tracks are all
 * `minmax(0, …)` or fixed, so a row can only truncate, never widen. What
 * leaves the row as the list narrows is chosen by *container* queries on
 * the list's own width, not the viewport's: the sidebar takes 16rem from a
 * viewport of `md` and up, so a viewport breakpoint would keep columns the
 * card has no room for.
 */
import * as React from "react"
import {
  Braces,
  CalendarClock,
  ChevronRight,
  CircleHelp,
  Hash,
  ToggleLeft,
  Type,
  type LucideIcon,
} from "lucide-react"
import { MetadataList, type MetadataItem } from "@/components/patterns/metadata-list"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import {
  formatShare,
  noValuesLabel,
  sortKeyColumns,
  valueShares,
  type NoValuesLabel,
  type ValueShare,
  type ValueShares,
} from "@/lib/column-profile"
import { typeFamily, type TypeFamily } from "@/lib/column-type"
import { formatCompactNumber, formatDate, formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { AssetColumn, AssetProfile, ColumnProfile } from "@/services/contracts/assets"
import { ALL, CountToggle } from "./count-toggle"

/** One row of the table: a catalog column with what the Iceberg table says about it. */
export type SchemaRow = {
  column: AssetColumn
  /** `null` when neither Iceberg nor the declared type settles it. */
  nullable: boolean | null
  /** The partition transform this column feeds, e.g. `day`. */
  partition: string | null
}

export type ProfileState =
  | { kind: "restricted" }
  | { kind: "loading" }
  | { kind: "error"; message: string; retry: () => void }
  | { kind: "unsupported"; reason: string }
  | { kind: "ready"; profile: Extract<AssetProfile, { supported: true }>; byName: Map<string, ColumnProfile> }

/** How many columns the table shows before the reader asks for more. */
const COLUMN_PAGE_SIZES = [25, 50, 100] as const

/** A table with more columns than this gets the filter box. */
const FILTER_ABOVE = 10

/** How an empty text is written wherever a value is listed; the Sample tab writes its empty cells the same way. */
export const EMPTY_VALUE = "(empty)"
const NOT_PROFILED = "Not profiled (type not supported)"
/** Said under a row's label where a bar would be, and in the opened column, when no value is listed. */
const NO_VALUE_LISTED = "No value is listed: the profile states a value's count only where it is exact."

/**
 * The columns to list: those whose name or description contains `query`,
 * cut to `limit`. `matched` is how many there are before the cut.
 */
export function visibleColumns<T extends { column: AssetColumn }>(
  rows: T[],
  query: string,
  limit: number
): { shown: T[]; matched: number } {
  const needle = query.trim().toLowerCase()
  const matching =
    needle === ""
      ? rows
      : rows.filter(
          (r) =>
            r.column.name.toLowerCase().includes(needle) ||
            (r.column.description ?? "").toLowerCase().includes(needle)
        )
  return { shown: matching.slice(0, limit), matched: matching.length }
}

// ── Layout ────────────────────────────────────────────────────────────────
//
// One string lays out the header and every row, so a label stays over its
// cells. Fixed tracks are `rem`, flexible ones `minmax(0, …fr)`: a row
// truncates, it never widens the list. Order of the tracks: glyph, name,
// [type], [values bar, nulls, distinct, range], chevron. Container widths
// (Tailwind's `@lg` 32rem, `@2xl` 42rem, `@3xl` 48rem, `@5xl` 64rem):
//   below @lg   the type sits under the name (a second grid line), with the
//               name's marks after it, so the name keeps its whole track; the
//               row is glyph, name, bar, null share
//   @lg         the type gets a track of its own, the marks sit beside the name
//   @2xl        the label beside the bar, and the distinct count
//   @3xl        the type's track is fixed at 9rem, which holds 20 characters of
//               mono `text-xs` (7.2px each): `Nullable(Float64)` is 17. Below
//               this the type is a fraction and truncates; the type matters
//               more on this tab than the range, so it is the range that waits
//   @5xl        the range, on a fixed 11.5rem track: two ISO dates and the dash
//               are 23 characters, about 10.4rem
// The bar and the label are two equal tracks inside the values cell, so every
// bar track is as long as every other at a given width. Arithmetic, with a
// row's width the container less 1.625rem (border and padding) and the gaps
// 0.75rem each, the rest shared by the flexible tracks (name 1.4 : values 2):
//   @3xl, 48rem   19.6rem free: name 8.1rem, values 11.5rem
//   58.5rem       (a 1280px viewport with the sidebar open; no range) name
//                 12.4rem, values 17.7rem
//   @5xl, 64rem   23.4rem free: name 9.6rem, values 13.8rem
//   68.5rem       (1440px) name 11.5rem, values 16.4rem
// Nothing a row sheds is lost: the opened column has it.
const ROW_BASE = "grid items-center gap-x-2 px-3 @lg:gap-x-3"
const TRACKS_WITH_STATS = [
  "grid-cols-[1.25rem_minmax(0,1.2fr)_minmax(0,1fr)_3.25rem_1rem]",
  "@lg:grid-cols-[1.25rem_minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,1fr)_6.5rem_1rem]",
  "@2xl:grid-cols-[1.25rem_minmax(0,1.2fr)_minmax(0,1fr)_minmax(0,2fr)_6.5rem_4.5rem_1rem]",
  "@3xl:grid-cols-[1.25rem_minmax(0,1.4fr)_9rem_minmax(0,2fr)_6.5rem_4.5rem_1rem]",
  "@5xl:grid-cols-[1.25rem_minmax(0,1.4fr)_9rem_minmax(0,2fr)_6.5rem_4.5rem_11.5rem_1rem]",
].join(" ")
const TRACKS_WITHOUT_STATS =
  "grid-cols-[1.25rem_minmax(0,1fr)_1rem] @lg:grid-cols-[1.25rem_minmax(0,1fr)_minmax(0,1.5fr)_1rem]"

/** A cell that stands beside the name below `@lg`: it spans the name's line and the type's. */
const TALL = "row-span-2 self-center @lg:row-span-1"

// ── One hue, stepped ──────────────────────────────────────────────────────
//
// Segments are one colour at stepped strengths, commonest strongest, on the
// muted track. A palette of five colours would read as five categories
// that mean something across columns; they do not. The `chart-3` token is
// the middle of the design system's blue scale, which holds against the
// muted track in light and dark (`chart-5` at full strength is too close
// to the dark track). A segment is told apart by its tooltip and by its
// line in the opened column, which uses the same step.
const STEPS = ["bg-chart-3", "bg-chart-3/80", "bg-chart-3/60", "bg-chart-3/45", "bg-chart-3/30"] as const
const stepClass = (i: number) => STEPS[Math.min(i, STEPS.length - 1)]

/** A share as a CSS width, to two decimals so `0.07` is `7%` and not `7.000000000000001%`. */
const widthOf = (share: number) => `${Number((share * 100).toFixed(2))}%`

const shownValue = (value: string) => (value === "" ? EMPTY_VALUE : value)

/** What a segment or a line says about its value, for the tooltip: `value · N rows (P%)`. */
const describeValue = (v: ValueShare) =>
  `${shownValue(v.value)} · ${formatNumber(v.count)} rows (${formatShare(v.share)})`

/** The glyph and the word for each kind of data; the Sample tab's headers use the same table. */
export const FAMILY: Record<TypeFamily, { label: string; icon: LucideIcon }> = {
  text: { label: "Text", icon: Type },
  number: { label: "Number", icon: Hash },
  time: { label: "Date or time", icon: CalendarClock },
  boolean: { label: "Boolean", icon: ToggleLeft },
  nested: { label: "Nested", icon: Braces },
  other: { label: "Other type", icon: CircleHelp },
}

function rangeOf(c: ColumnProfile): string | null {
  return c.min != null && c.max != null ? (c.min === c.max ? c.min : `${c.min} – ${c.max}`) : null
}

/**
 * Null share as a thin meter plus its number. The number carries the
 * meaning; the bar only lets the eye find the gappy columns in a long list,
 * and goes first when the row is short of room.
 */
function NullMeter({ fraction }: { fraction: number }) {
  return (
    <span className="flex items-center gap-2">
      <span className="hidden h-1.5 w-12 overflow-hidden rounded-full bg-muted @lg:block" aria-hidden>
        <span
          className="block h-full rounded-full bg-foreground/40"
          style={{ width: `${Math.min(fraction, 1) * 100}%` }}
        />
      </span>
      <span className="tabular-nums">{formatShare(fraction)}</span>
    </span>
  )
}

/** What the stats were computed over, or why there are none. */
export function ProfileNote({ state }: { state: ProfileState }) {
  if (state.kind === "loading") return <span>Profiling columns…</span>
  if (state.kind === "restricted") {
    return <span>Column statistics are read from the data, so they need the query:read permission.</span>
  }
  if (state.kind === "unsupported") return <span>No column statistics: {state.reason}</span>
  if (state.kind === "error") {
    return (
      <span className="flex items-center gap-2">
        Column statistics failed to load: {state.message}
        <Button size="sm" variant="ghost" onClick={state.retry}>
          Retry
        </Button>
      </span>
    )
  }
  const p = state.profile
  return (
    <span>
      Statistics over {formatNumber(p.rowsProfiled)} rows of <span className="font-mono">{p.source}</span>
      {p.sourceKind === "iceberg" ? " (Iceberg)" : ""}
      {p.sampled ? ` (first ${formatNumber(p.rowLimit)} only)` : ""}
      {p.columnsCapped ? " · some columns left out" : ""}. Masking and row filters apply.
    </span>
  )
}

/**
 * The row's picture of the column: one segment per listed value, as wide as
 * its share of the rows profiled, on the bare track; beside it, at the
 * widths that have room, the commonest value and its share. The meaning is
 * not in the colour: each segment's tooltip says its value, rows and
 * share, the bar's label lists them, and the opened column has every line.
 *
 * The bar and the label are two tracks of fixed proportions (`1fr 1fr`, both
 * `minmax(0, …)`), so the track is as long on every row at a given width
 * whatever the commonest value is: a bar's length is only ever read against
 * the same track. The value truncates inside its track and the share sits
 * at the track's right edge, so a longer value never takes room from the bar.
 */
function ValueBar({ shares }: { shares: ValueShares }) {
  const top = shares.values[0]
  if (!top) return null
  return (
    <div className="grid grid-cols-1 items-center gap-x-2 @2xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div
        role="img"
        aria-label={`Most frequent values: ${shares.values.map((v) => `${shownValue(v.value)} ${formatShare(v.share)}`).join(", ")}`}
        className="flex h-2 w-full overflow-hidden rounded-full bg-muted"
      >
        {shares.values.map((v, i) => (
          <span
            key={`${i}:${v.value}`}
            title={describeValue(v)}
            className={cn("h-full shrink-0", v.share > 0 && "min-w-px", stepClass(i))}
            style={{ width: widthOf(v.share) }}
          />
        ))}
      </div>
      <span
        className="hidden min-w-0 items-baseline justify-between gap-2 text-xs @2xl:flex"
        title={describeValue(top)}
      >
        <span className="min-w-0 truncate font-mono">{shownValue(top.value)}</span>
        <span className="shrink-0 text-right tabular-nums text-muted-foreground">{formatShare(top.share)}</span>
      </span>
    </div>
  )
}

/**
 * What stands where the bar would be for a column with no listed value. An
 * empty list is not "unique" (see `noValuesLabel`): the label says what the
 * counts support, and its tooltip says why no value is listed. It spans the
 * whole cell; it is not a bar and has no track.
 */
function NoValues({ label }: { label: NoValuesLabel }) {
  return (
    <span className="text-xs text-muted-foreground" title={NO_VALUE_LISTED}>
      {label}
    </span>
  )
}

/** The four statistic cells of a row, or the one message that stands for them. */
function StatCells({
  state,
  column,
  shares,
  noValues,
}: {
  state: ProfileState
  column: ColumnProfile | undefined
  shares: ValueShares | null
  /** What to say where there is no bar; `null` when there is no profile of the column's rows to say it from. */
  noValues: NoValuesLabel | null
}) {
  if (state.kind === "loading") {
    return (
      <>
        <div role="cell" className={cn("min-w-0", TALL)}>
          <Skeleton className="h-3.5 w-full max-w-40" />
        </div>
        <div role="cell" className={TALL}>
          <Skeleton className="h-3.5 w-10" />
        </div>
        <div role="cell" className="hidden self-center @2xl:block">
          <Skeleton className="h-3.5 w-8" />
        </div>
        <div role="cell" className="hidden self-center @5xl:block">
          <Skeleton className="h-3.5 w-16" />
        </div>
      </>
    )
  }
  if (!column || !column.profiled) {
    return (
      <div
        role="cell"
        className={cn(
          "col-span-2 line-clamp-2 min-w-0 text-xs text-muted-foreground @lg:line-clamp-1 @2xl:col-span-3 @5xl:col-span-4",
          TALL
        )}
      >
        {column ? NOT_PROFILED : "—"}
      </div>
    )
  }
  const range = rangeOf(column)
  return (
    <>
      <div role="cell" className={cn("min-w-0", TALL)}>
        {shares && shares.values.length > 0 ? (
          <ValueBar shares={shares} />
        ) : noValues ? (
          <NoValues label={noValues} />
        ) : (
          <span className="text-xs text-muted-foreground">—</span>
        )}
      </div>
      <div role="cell" className={cn("text-xs", TALL)}>
        {column.nullFraction == null ? "—" : <NullMeter fraction={column.nullFraction} />}
      </div>
      <div role="cell" className="hidden self-center text-xs tabular-nums @2xl:block" title="Approximate">
        {column.distinctCount == null ? "—" : `≈ ${formatCompactNumber(column.distinctCount)}`}
      </div>
      <div
        role="cell"
        className="hidden min-w-0 self-center truncate font-mono text-xs @5xl:block"
        title={range ?? undefined}
      >
        {range ?? "—"}
      </div>
    </>
  )
}

/**
 * One line of the opened column: a value, a bar as wide as its share (at
 * the step its segment has), its count and its share. The tracks are the
 * same on every line so the numbers line up.
 */
function ValueLine({
  label,
  title,
  mono,
  count,
  share,
  fill,
}: {
  label: string
  title?: string
  /** A value reads in mono; "Other values" and "Null" are words, in the plain tone. */
  mono: boolean
  count: number | null
  share: number
  fill: string
}) {
  return (
    <li className="grid grid-cols-[minmax(0,1.2fr)_minmax(2.5rem,1fr)_4rem_3.25rem] items-center gap-x-2 text-xs">
      <span className={cn("truncate", mono ? "font-mono" : "text-muted-foreground")} title={title ?? label}>
        {label}
      </span>
      <span className="h-2 overflow-hidden rounded-full bg-muted" aria-hidden>
        <span
          className={cn("block h-full rounded-full", share > 0 && "min-w-px", fill)}
          style={{ width: widthOf(share) }}
        />
      </span>
      <span className="text-right tabular-nums">{formatNumber(count)}</span>
      <span className="text-right tabular-nums text-muted-foreground">{formatShare(share)}</span>
    </li>
  )
}

/** The values half of the opened column: every listed value, then the rest and the nulls. */
function ValueLines({
  state,
  column,
  shares,
}: {
  state: Extract<ProfileState, { kind: "loading" | "ready" }>
  column: ColumnProfile | undefined
  shares: ValueShares | null
}) {
  if (state.kind === "loading") {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-3.5 w-full" />
        <Skeleton className="h-3.5 w-full" />
        <Skeleton className="h-3.5 w-3/4" />
      </div>
    )
  }
  if (!column) return <p className="text-xs text-muted-foreground">This column is not in the profile.</p>
  if (!column.profiled) return <p className="text-xs text-muted-foreground">{NOT_PROFILED}</p>
  if (!shares) return <p className="text-xs text-muted-foreground">No rows were profiled.</p>
  const { profile } = state
  return (
    <>
      {shares.values.length === 0 ? (
        <p className="mb-1.5 text-xs text-muted-foreground">{NO_VALUE_LISTED}</p>
      ) : null}
      <ul className="flex flex-col gap-1.5">
        {shares.values.map((v, i) => (
          <ValueLine
            key={`${i}:${v.value}`}
            label={shownValue(v.value)}
            title={describeValue(v)}
            mono
            count={v.count}
            share={v.share}
            fill={stepClass(i)}
          />
        ))}
        {shares.otherShare !== null && shares.otherShare > 0 ? (
          <ValueLine
            label="Other values"
            mono={false}
            count={shares.otherCount}
            share={shares.otherShare}
            fill="bg-foreground/25"
          />
        ) : null}
        {shares.nullShare !== null && shares.nullShare > 0 ? (
          <ValueLine
            label="Null"
            mono={false}
            count={shares.nullCount}
            share={shares.nullShare}
            fill="bg-foreground/25"
          />
        ) : null}
      </ul>
      {profile.sampled ? (
        <p className="mt-2 text-xs text-muted-foreground">
          Shares are of the rows profiled (first {formatNumber(profile.rowLimit)} only); the table may hold more.
        </p>
      ) : null}
    </>
  )
}

/** What the opened column states about itself; each fact is there only when it has something to say. */
function factsOf(row: SchemaRow, column: ColumnProfile | undefined, sortKey: boolean): MetadataItem[] {
  const c = row.column
  const mono = "break-all font-mono text-xs"
  const items: MetadataItem[] = [
    { label: "Type", value: <span className={mono}>{c.dataType}</span> },
    { label: "Can be null", value: row.nullable === null ? "Not known" : row.nullable ? "Yes" : "No" },
  ]
  if (column?.profiled) {
    if (column.nullCount != null || column.nullFraction != null) {
      items.push({
        label: "Nulls",
        value: `${column.nullCount != null ? formatNumber(column.nullCount) : "—"}${
          column.nullFraction != null ? ` (${formatShare(column.nullFraction)})` : ""
        }`,
      })
    }
    if (column.distinctCount != null) {
      items.push({ label: "Distinct", value: `≈ ${formatNumber(column.distinctCount)}` })
    }
    const range = rangeOf(column)
    if (range !== null) items.push({ label: "Range", value: <span className={mono}>{range}</span> })
  }
  if (row.partition) items.push({ label: "Partitioned by", value: `This column (${row.partition})` })
  if (sortKey) items.push({ label: "Sorted by", value: "This column" })
  // The row's marks can be cut short on a narrow screen; the opened column says them in full.
  if (c.masked) items.push({ label: "Masked", value: "Yes" })
  if (c.classification) {
    items.push({ label: "Classification", value: <ClassificationBadge classification={c.classification} /> })
  }
  if (c.description) items.push({ label: "Description", value: c.description })
  return items
}

/**
 * The opened column: its values on the left where there is room, its facts
 * beside or under them. The Sample tab's inspector opens the same one.
 */
export function ColumnDetail({
  row,
  state,
  column,
  shares,
  sortKey,
}: {
  row: SchemaRow
  state: ProfileState
  column: ColumnProfile | undefined
  shares: ValueShares | null
  sortKey: boolean
}) {
  const name = row.column.name
  // Without a profile (no permission, unsupported, failed) the column still opens, on the facts that need none.
  const withValues = state.kind === "ready" || state.kind === "loading"
  return (
    <div className={cn("grid gap-x-8 gap-y-4", withValues && "@3xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]")}>
      {withValues ? (
        <div role="group" aria-label={`Values of ${name}`} className="min-w-0">
          <p className="mb-1.5 text-xs font-medium text-muted-foreground">Most frequent values</p>
          <ValueLines state={state} column={column} shares={shares} />
        </div>
      ) : null}
      <div role="group" aria-label={`Facts about ${name}`} className="min-w-0">
        <MetadataList items={factsOf(row, column, sortKey)} columns={withValues ? 2 : 3} />
      </div>
    </div>
  )
}

/**
 * What a column is marked as: inactive at the source (`SRC-8`), masked, its
 * classification, partition, sort key.
 * A row renders these twice and CSS shows one: beside the name from `@lg` up,
 * and below it on the type's line otherwise. Below `@lg` the name's track is
 * a third of a phone's card, so marks beside it (they never shrink) left a
 * name like `plnt` as "pl…"; on the type's line it is the type that truncates.
 * The copy that is not shown is `display: none`, so nothing reads twice.
 */
function Marks({ row, sortKey, className }: { row: SchemaRow; sortKey: boolean; className: string }) {
  const c = row.column
  if (!c.masked && !c.classification && !row.partition && !sortKey && !c.inactiveSince) return null
  return (
    <span className={cn("shrink-0 items-center gap-1.5", className)}>
      {c.inactiveSince ? (
        <Pill tone="neutral" title="The source no longer has this column. Its old values are kept.">
          inactive since {formatDate(c.inactiveSince, { month: "short" })}
        </Pill>
      ) : null}
      {c.masked ? <Pill tone="warning">masked</Pill> : null}
      {c.classification ? <ClassificationBadge classification={c.classification} /> : null}
      {row.partition ? (
        <Pill tone="neutral" title="The table is partitioned by this column">
          partition · {row.partition}
        </Pill>
      ) : null}
      {sortKey ? (
        <Pill tone="neutral" title="The table's sorting key names this column">
          sort key
        </Pill>
      ) : null}
    </span>
  )
}

function ColumnRow({
  row,
  state,
  showStats,
  sortKey,
  open,
  onToggle,
}: {
  row: SchemaRow
  state: ProfileState
  showStats: boolean
  sortKey: boolean
  open: boolean
  onToggle: () => void
}) {
  const panelId = React.useId()
  const c = row.column
  const family = FAMILY[typeFamily(c.dataType)]
  const Glyph = family.icon
  const profiled = state.kind === "ready" ? state.byName.get(c.name) : undefined
  const shares = state.kind === "ready" ? valueShares(profiled, state.profile.rowsProfiled) : null
  const noValues = state.kind === "ready" ? noValuesLabel(profiled, state.profile.rowsProfiled) : null
  return (
    <>
      {/* The button is the control for the keyboard and for assistive technology. The row takes the
          press as well, so the whole line works for a pointer; a press on the button bubbles here
          once, so the column toggles once. */}
      <div
        role="row"
        data-open={open ? "" : undefined}
        onClick={onToggle}
        className={cn(
          ROW_BASE,
          showStats ? TRACKS_WITH_STATS : TRACKS_WITHOUT_STATS,
          "min-h-13 cursor-pointer border-b py-1.5 transition-colors last:border-b-0 hover:bg-muted/50 data-open:bg-muted/30 @lg:min-h-9"
        )}
      >
        <div role="cell" className={TALL}>
          <span
            role="img"
            aria-label={family.label}
            title={family.label}
            className="grid size-5 place-items-center rounded-md bg-muted text-muted-foreground"
          >
            <Glyph className="size-3" aria-hidden />
          </span>
        </div>
        <div role="cell" className="min-w-0 self-end @lg:self-center">
          <div className="flex h-5 min-w-0 items-center gap-1.5 overflow-hidden">
            <button
              type="button"
              aria-expanded={open}
              aria-controls={panelId}
              title={c.name}
              className="min-w-0 truncate rounded-sm text-left font-mono text-xs font-medium outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              {c.name}
            </button>
            <Marks row={row} sortKey={sortKey} className="hidden @lg:flex" />
          </div>
        </div>
        {/* Below `@lg` this is the second line of the name's track: the type, which truncates, then
            the marks. Its line is as tall as the name's so rows stay of one height. */}
        <div
          role="cell"
          className="col-start-2 row-start-2 flex h-5 min-w-0 items-center gap-1.5 self-start overflow-hidden @lg:col-start-auto @lg:row-start-auto @lg:h-auto @lg:self-center"
        >
          <span
            title={c.dataType}
            className="min-w-0 truncate font-mono text-[11px] leading-4 text-muted-foreground @lg:text-xs"
          >
            {c.dataType}
          </span>
          <Marks row={row} sortKey={sortKey} className="flex @lg:hidden" />
        </div>
        {showStats ? <StatCells state={state} column={profiled} shares={shares} noValues={noValues} /> : null}
        <div role="cell" className={cn("flex justify-end", TALL)}>
          <ChevronRight
            className={cn("size-4 text-muted-foreground transition-transform", open && "rotate-90")}
            aria-hidden
          />
        </div>
      </div>
      {open ? (
        <div role="row" className="border-b last:border-b-0">
          <div role="cell" id={panelId} className="bg-muted/20 px-3 py-3">
            <ColumnDetail row={row} state={state} column={profiled} shares={shares} sortKey={sortKey} />
          </div>
        </div>
      ) : null}
    </>
  )
}

/** The column labels over the rows; they shed with the cells they label. */
function ColumnHeader({ showStats }: { showStats: boolean }) {
  return (
    <div
      role="row"
      className={cn(
        ROW_BASE,
        showStats ? TRACKS_WITH_STATS : TRACKS_WITHOUT_STATS,
        "border-b bg-muted/30 py-2 text-xs font-medium text-muted-foreground"
      )}
    >
      <div role="columnheader">
        <span className="sr-only">Kind</span>
      </div>
      <div role="columnheader">Column</div>
      <div role="columnheader" className="hidden @lg:block">
        Type
      </div>
      {showStats ? (
        <>
          <div role="columnheader">Top values</div>
          <div role="columnheader">Nulls</div>
          <div role="columnheader" className="hidden @2xl:block">
            Distinct
          </div>
          <div role="columnheader" className="hidden @5xl:block">
            Range
          </div>
        </>
      ) : null}
      <div role="columnheader">
        <span className="sr-only">Details</span>
      </div>
    </div>
  )
}

/**
 * The "Columns" card: the filter and page size for a wide table, the list,
 * and the note saying what the statistics were computed over.
 */
export function ColumnsCard({
  rows,
  profile,
  sortingKey,
}: {
  rows: SchemaRow[]
  profile: ProfileState
  /** `AssetDetail.storage.sortingKey`: the engine's key, which may name columns or hold expressions. */
  sortingKey?: string | null
}) {
  const showStats = profile.kind === "ready" || profile.kind === "loading"
  const [limit, setLimit] = React.useState<number>(COLUMN_PAGE_SIZES[0])
  const [query, setQuery] = React.useState("")
  // Held here, not in each row, so a column stays open while the filter narrows the list and widens it again.
  const [open, setOpen] = React.useState<ReadonlySet<string>>(() => new Set())
  const toggle = (name: string) =>
    setOpen((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  // A table that fits the first page needs no page size; one of a dozen columns is already worth a filter.
  const paged = rows.length > COLUMN_PAGE_SIZES[0]
  const filterable = rows.length > FILTER_ABOVE
  // Only the sizes that would cut this table short, then all of it.
  const sizes = [...COLUMN_PAGE_SIZES.filter((n) => n < rows.length), ALL]
  const { shown, matched } = visibleColumns(rows, query, paged ? limit : ALL)
  const sortKeys = sortKeyColumns(
    sortingKey,
    rows.map((r) => r.column.name)
  )
  const total = `${rows.length} column${rows.length === 1 ? "" : "s"}`
  const summary =
    query.trim() !== ""
      ? `${matched} of ${total} match "${query.trim()}"${shown.length < matched ? `, showing the first ${shown.length}` : ""}.`
      : shown.length < rows.length
        ? `Showing the first ${shown.length} of ${total}, in table order.`
        : `${total}, in table order.`

  return (
    <SectionCard size="sm" title="Columns" description={summary}>
      <div className="flex flex-col gap-2">
        {/* In the body, not the card's header action: that slot cannot shrink, so a filter beside a
            page-size toggle pushed past the card at phone width. */}
        {filterable ? (
          <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
            <Input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter columns…"
              aria-label="Filter columns"
              className="h-8 w-full sm:w-56"
            />
            {paged ? (
              <CountToggle
                label="Show"
                ariaLabel="Columns to show"
                options={sizes}
                value={limit}
                onChange={setLimit}
              />
            ) : null}
          </div>
        ) : null}
        {rows.length === 0 ? (
          <EmptyState title="No columns registered" className="py-4" />
        ) : shown.length === 0 ? (
          <EmptyState title={`No column matches "${query.trim()}"`} className="py-4" />
        ) : (
          <>
            <div className="@container overflow-hidden rounded-lg border border-border">
              <div role="table" aria-label="Columns">
                <div role="rowgroup">
                  <ColumnHeader showStats={showStats} />
                </div>
                <div role="rowgroup">
                  {shown.map((row) => (
                    <ColumnRow
                      key={row.column.name}
                      row={row}
                      state={profile}
                      showStats={showStats}
                      sortKey={sortKeys.has(row.column.name)}
                      open={open.has(row.column.name)}
                      onToggle={() => toggle(row.column.name)}
                    />
                  ))}
                </div>
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              <ProfileNote state={profile} />
            </p>
          </>
        )}
      </div>
    </SectionCard>
  )
}
