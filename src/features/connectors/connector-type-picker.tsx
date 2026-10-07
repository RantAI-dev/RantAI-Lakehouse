"use client"

import * as React from "react"
import {
  ArrowDownToLineIcon,
  ArrowLeftRightIcon,
  ArrowUpFromLineIcon,
  CheckIcon,
  DatabaseIcon,
  ExternalLinkIcon,
  GitCompareArrowsIcon,
  GlobeIcon,
  HardDriveIcon,
  LeafIcon,
  PlugIcon,
  RadioTowerIcon,
  SearchIcon,
  ServerIcon,
  SheetIcon,
  TriangleAlertIcon,
  type LucideIcon,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { BrandMark } from "@/components/patterns/brand-mark"
import { brandMarkForTypeName } from "@/lib/connectors/brand-marks"
import type { Connector, ConnectorType, IngestAdapter } from "@/services/contracts/connectors"
import { cn } from "@/lib/utils"

type Category = "database" | "cdc" | "files" | "apps" | "soon"

const CATEGORIES: { id: Category; label: string; description: string }[] = [
  { id: "database", label: "Databases", description: "Read tables or collections in batches, on demand or on a schedule." },
  { id: "cdc", label: "Change data capture", description: "Stream every insert, update and delete as it happens." },
  { id: "files", label: "Files and storage", description: "Pick up CSV files from a bucket or a remote directory." },
  { id: "apps", label: "APIs and streams", description: "Pull from HTTP endpoints, spreadsheets or message topics." },
  { id: "soon", label: "Coming soon", description: "Listed so you know they are planned. They cannot be created yet." },
]

/** How each adapter is presented: its group, icon and one-line summary. */
const ADAPTER_META: Record<IngestAdapter, { category: Category; icon: LucideIcon; blurb: string }> = {
  sql: { category: "database", icon: DatabaseIcon, blurb: "Batch reads of selected tables" },
  mongodb: { category: "database", icon: LeafIcon, blurb: "Batch reads of selected collections" },
  cdc: { category: "cdc", icon: GitCompareArrowsIcon, blurb: "Row-level changes through Debezium" },
  files: { category: "files", icon: HardDriveIcon, blurb: "CSV objects in an S3-compatible bucket" },
  sftp: { category: "files", icon: ServerIcon, blurb: "CSV files in a remote directory" },
  rest: { category: "apps", icon: GlobeIcon, blurb: "Paginated JSON endpoints" },
  sheets: { category: "apps", icon: SheetIcon, blurb: "Rows from a Google Sheets tab" },
  kafka: { category: "apps", icon: RadioTowerIcon, blurb: "Micro-batches from topics" },
}

function metaFor(type: ConnectorType) {
  if (!type.supported || !type.adapter) {
    return { category: "soon" as const, icon: PlugIcon, blurb: "Not available yet" }
  }
  return ADAPTER_META[type.adapter] ?? { category: "apps" as const, icon: PlugIcon, blurb: "" }
}

/**
 * The type's own product mark when the set publishes one (see
 * `@/lib/connectors/brand-marks`), else the adapter's generic icon. Tile and
 * strip both render this so they cannot differ. On the primary-coloured chip
 * (`onPrimary`) a mark takes the chip's foreground like the generic icons.
 */
function TypeIcon({
  type,
  fallback: Fallback,
  onPrimary = false,
}: {
  type: ConnectorType
  fallback: LucideIcon
  onPrimary?: boolean
}) {
  const mark = brandMarkForTypeName(type.name)
  if (!mark) return <Fallback className="size-4" aria-hidden="true" />
  return (
    <BrandMark
      paths={mark.paths}
      viewBox={mark.viewBox}
      hex={mark.hex}
      foregroundInDark={mark.needsForegroundInDark}
      tone={onPrimary ? "inherit" : "brand"}
      className="size-[18px]"
    />
  )
}

/**
 * The connector-type step's picker: one selectable card per registry row,
 * grouped by what kind of source it is. Unsupported rows stay visible
 * (a real roadmap entry, never hidden) but cannot be selected.
 */
export function ConnectorTypePicker({
  types,
  value,
  onChange,
}: {
  types: ConnectorType[]
  value: string | null
  onChange: (name: string) => void
}) {
  const [query, setQuery] = React.useState("")
  const needle = query.trim().toLowerCase()
  const selected = types.find((t) => t.name === value) ?? null

  const groups = CATEGORIES.map((category) => ({
    ...category,
    items: types.filter((t) => {
      const meta = metaFor(t)
      if (meta.category !== category.id) return false
      if (!needle) return true
      return [t.name, meta.blurb, category.label].some((text) => text.toLowerCase().includes(needle))
    }),
  })).filter((group) => group.items.length > 0)

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-medium">Connector type</h3>
          <p className="text-xs text-muted-foreground">Where the data comes from. This decides the connection form in the next step.</p>
        </div>
        <div className="relative w-full sm:w-56">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            id="connector-type-search"
            aria-label="Search connector types"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search types"
            className="pl-8"
            autoComplete="off"
          />
        </div>
      </div>

      <div role="radiogroup" aria-label="Connector type" className="space-y-5">
        {groups.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-sm text-muted-foreground">
            No connector type matches &ldquo;{query.trim()}&rdquo;.
          </p>
        ) : null}
        {groups.map((group) => (
          <section key={group.id} className="space-y-2">
            <div className="flex flex-wrap items-baseline gap-x-2">
              <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{group.label}</h4>
              <span className="text-xs text-muted-foreground/80">{group.description}</span>
            </div>
            <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
              {group.items.map((type) => (
                <TypeCard
                  key={type.name}
                  type={type}
                  checked={type.name === value}
                  onSelect={() => onChange(type.name)}
                />
              ))}
            </div>
          </section>
        ))}
      </div>

      {selected?.adapter === "sheets" ? (
        // rust/migrations/0035_connector_type.sql seeds Google Sheets
        // supported = true (the adapter and wizard exist), but this build's
        // sheets ingest adapter reports unsupported for every run (WS3 item
        // 23) -- said here so nobody picks it on the strength of the
        // registry row alone and finds out only after clicking Run.
        <p className="flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-300">
          <TriangleAlertIcon className="mt-px size-3.5 shrink-0" />
          Google Sheets connections can be configured and tested, but this build cannot move data for this type
          yet. A triggered run reports that it is unsupported instead of failing silently.
        </p>
      ) : null}
      {selected?.docsUrl ? (
        <a
          href={selected.docsUrl}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 text-xs text-primary underline-offset-4 hover:underline"
        >
          {selected.name} documentation
          <ExternalLinkIcon className="size-3" />
        </a>
      ) : null}
    </div>
  )
}

function TypeCard({
  type,
  checked,
  onSelect,
}: {
  type: ConnectorType
  checked: boolean
  onSelect: () => void
}) {
  const meta = metaFor(type)
  const disabled = !type.supported
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      disabled={disabled}
      title={disabled ? "Not yet supported" : undefined}
      onClick={onSelect}
      className={cn(
        "group relative flex w-full items-start gap-3 rounded-lg border bg-card px-3 py-2.5 text-left transition-colors outline-none",
        "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
        checked ? "border-primary bg-primary/5 ring-1 ring-primary" : "border-border hover:border-foreground/25 hover:bg-muted/40",
        disabled && "cursor-not-allowed border-dashed bg-transparent opacity-60 hover:border-border hover:bg-transparent"
      )}
    >
      <span
        className={cn(
          "flex size-8 shrink-0 items-center justify-center rounded-md",
          checked ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"
        )}
      >
        <TypeIcon type={type} fallback={meta.icon} onPrimary={checked} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium">{type.name}</span>
          {disabled ? (
            <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
              Soon
            </Badge>
          ) : type.adapter === "sheets" ? (
            <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
              Test only
            </Badge>
          ) : null}
        </span>
        <span className="mt-0.5 block text-xs text-muted-foreground">{meta.blurb}</span>
      </span>
      {checked ? <CheckIcon className="absolute top-2 right-2 size-3.5 text-primary" aria-hidden /> : null}
    </button>
  )
}

/**
 * The chosen type as a compact strip, for the steps after the picker, with
 * a way back to change it — or, without `onChange` (an existing
 * connector, whose type is fixed), a note saying it cannot change.
 */
export function SelectedTypeSummary({ type, onChange }: { type: ConnectorType; onChange?: () => void }) {
  const meta = metaFor(type)
  return (
    <div className="flex items-center gap-3 rounded-lg border border-border bg-muted/30 px-3 py-2.5">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary text-primary-foreground">
        <TypeIcon type={type} fallback={meta.icon} onPrimary />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{type.name}</span>
        <span className="block truncate text-xs text-muted-foreground">{meta.blurb}</span>
      </span>
      {onChange ? (
        <button
          type="button"
          onClick={onChange}
          className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-primary outline-none hover:bg-primary/10 focus-visible:ring-[3px] focus-visible:ring-ring/50"
        >
          Change type
        </button>
      ) : (
        <span className="shrink-0 text-xs text-muted-foreground">Type is fixed</span>
      )}
    </div>
  )
}

const DIRECTIONS: {
  value: Connector["direction"]
  label: string
  description: string
  icon: LucideIcon
}[] = [
  { value: "source", label: "Source", description: "Data flows into the lakehouse", icon: ArrowDownToLineIcon },
  { value: "sink", label: "Sink", description: "Data flows out of the lakehouse", icon: ArrowUpFromLineIcon },
  { value: "bidirectional", label: "Bidirectional", description: "Both directions", icon: ArrowLeftRightIcon },
]

/** Three-way choice for `Connector["direction"]`, as compact cards. */
export function DirectionPicker({
  value,
  onChange,
}: {
  value: Connector["direction"]
  onChange: (value: Connector["direction"]) => void
}) {
  return (
    <div role="radiogroup" aria-label="Direction" className="grid gap-2 sm:grid-cols-3">
      {DIRECTIONS.map((option) => {
        const checked = option.value === value
        const Icon = option.icon
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={checked}
            onClick={() => onChange(option.value)}
            className={cn(
              "flex items-center gap-2.5 rounded-lg border px-3 py-2 text-left transition-colors outline-none",
              "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
              checked ? "border-primary bg-primary/5 ring-1 ring-primary" : "border-border hover:bg-muted/40"
            )}
          >
            <Icon className={cn("size-4 shrink-0", checked ? "text-primary" : "text-muted-foreground")} />
            <span className="min-w-0">
              <span className="block text-sm font-medium">{option.label}</span>
              <span className="block text-xs text-muted-foreground">{option.description}</span>
            </span>
          </button>
        )
      })}
    </div>
  )
}
