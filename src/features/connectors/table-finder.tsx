"use client"

import * as React from "react"
import { observeElementRect, useVirtualizer } from "@tanstack/react-virtual"
import { LoaderIcon, SearchIcon } from "lucide-react"
import { ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { formatBytes, formatDateTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { ColumnPreviews, PreviewEntry } from "./use-discovery"
import { useDiscoveryList } from "./use-discovery"
import type { Dial, DiscoveryAnswer, DiscoveryObject, DiscoveryRequest } from "@/services/contracts/connectors"

/**
 * The one "Find" flow for every connector type (`SRC-13` decisions D1 to D9): the source's
 * own list to pick from instead of typing names, and the columns of a listed object before
 * it is chosen.
 *
 * What is listed, by type: a SQL source's schemas, then the base tables of one (D7, D8);
 * object storage and SFTP files under a glob (D2); the endpoints a REST connector declares
 * (D3); a MongoDB database's collections; a Kafka broker's topics (D4); an Oracle login's
 * schemas, then its tables (D5).
 */
export type FinderKind = "sql" | "oracle" | "files" | "sftp" | "rest" | "mongodb" | "kafka" | "saas"

/** The finder a connector type gets, or `null` for a type that cannot list (Google Sheets). */
export function finderKind(adapter: string, dial: Dial | null | undefined): FinderKind | null {
  switch (adapter) {
    case "sql":
    case "cdc":
      // A MongoDB change-capture connector lists collections, like a batch MongoDB one (`SRC-4` slice 5).
      if (dial?.driver === "mongodb") return "mongodb"
      return dial?.driver === "oracle" ? "oracle" : "sql"
    case "files":
    case "sftp":
    case "rest":
    case "mongodb":
    case "kafka":
    case "saas":
      return adapter
    default:
      return null
  }
}

/** The kinds the API answers in the first call (no orchestrator job): the wait is for the source only. */
const ANSWERS_INLINE: ReadonlySet<FinderKind> = new Set(["sql", "files", "rest"])

/** What the finder calls the things it lists, and the button that finds them. */
export const FINDER_WORDS: Record<FinderKind, { many: string; find: string }> = {
  sql: { many: "tables", find: "Find tables" },
  oracle: { many: "tables", find: "Find tables" },
  files: { many: "files", find: "Find files" },
  sftp: { many: "files", find: "Find files" },
  rest: { many: "endpoints", find: "Find endpoints" },
  mongodb: { many: "collections", find: "Find collections" },
  kafka: { many: "topics", find: "Find topics" },
  saas: { many: "objects", find: "Find objects" },
}

/**
 * A list of more than this many names is searched on the server (D9), so the console
 * neither holds nor filters a huge list on its own; a shorter, whole one is filtered here.
 * The orchestrator's kinds search on the server only when the list was cut, because each
 * search there is a job.
 */
export const SERVER_SEARCH_ABOVE = 500
/** Rows are drawn all at once up to this many, and only the rows in view beyond it (D9). */
export const WINDOW_ABOVE = 200
const ROW_PX = 28
/** The height of the scrolling list (`h-64`, 16rem): what the virtualizer draws for before the browser has laid it out. */
const WINDOW_PX = 256
const SEARCH_DEBOUNCE_MS = 400

function secondsOf(ms: number): number {
  return Math.floor(ms / 1000)
}

/** The sentence under a failed listing: the API's fixed reason, never the source's text. */
function failureReason(answer: DiscoveryAnswer): string {
  return answer.reason ?? "The source could not be listed."
}

function ObjectRow({
  object,
  inputType,
  checked,
  disabled,
  onToggle,
  onColumns,
}: {
  object: DiscoveryObject
  inputType: "checkbox" | "radio"
  checked: boolean
  disabled: boolean
  onToggle: () => void
  onColumns: () => void
}) {
  const id = `discover-${object.name}`
  const nameId = `${id}-name`
  return (
    <>
      <label
        htmlFor={id}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-sm hover:bg-muted/50"
      >
        <input id={id} type={inputType} checked={checked} disabled={disabled} onChange={onToggle} />
        {/* Slice 5 review BLOCKER 14: a vendor whose names are ids (SharePoint lists) gives a label to read. */}
        <span id={nameId} className={cn("min-w-0 flex-1 truncate text-xs", object.label ? "" : "font-mono")} title={object.label ? object.name : undefined}>
          {object.label ?? object.name}
        </span>
        {object.kind === "file" ? (
          <span className="shrink-0 text-xs text-muted-foreground">
            {formatBytes(object.size)}
            {object.modifiedAt ? ` · ${formatDateTime(object.modifiedAt)}` : ""}
          </span>
        ) : null}
        {/* SRC-3 decision 6: say how a business-app object loads, so nobody finds out from a slow run. */}
        {object.load ? (
          <span data-testid="object-load" className="shrink-0 text-xs text-muted-foreground">
            {object.load}
          </span>
        ) : null}
      </label>
      {/* Described by the name, not labelled with it: the checkbox keeps the one label that names the object. */}
      <Button type="button" size="xs" variant="ghost" aria-describedby={nameId} onClick={onColumns}>
        Columns
      </Button>
    </>
  )
}

type RowsProps = {
  objects: DiscoveryObject[]
  inputType: "checkbox" | "radio"
  selected: ReadonlySet<string>
  disabled: boolean
  onToggle: (name: string, label?: string) => void
  onColumns: (name: string) => void
}

function PlainRows({ objects, inputType, selected, disabled, onToggle, onColumns }: RowsProps) {
  return (
    <ul className="max-h-64 space-y-0.5 overflow-y-auto">
      {objects.map((object) => (
        <li key={object.name} className="flex items-center gap-1">
          <ObjectRow
            object={object}
            inputType={inputType}
            checked={selected.has(object.name)}
            disabled={disabled}
            onToggle={() => onToggle(object.name, object.label)}
            onColumns={() => onColumns(object.name)}
          />
        </li>
      ))}
    </ul>
  )
}

/** Only the rows in view are in the page (`@tanstack/react-virtual`, already a dependency): a list of 10,000 names stays light (D9). */
function WindowedRows({ objects, inputType, selected, disabled, onToggle, onColumns }: RowsProps) {
  const scroller = React.useRef<HTMLDivElement>(null)
  // eslint-disable-next-line react-hooks/incompatible-library -- the same hook `data-table.tsx` uses; the component is a plain list.
  const virtualizer = useVirtualizer({
    count: objects.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW_PX,
    overscan: 8,
    // The list has a fixed CSS height, so a measured height of 0 means "not laid out yet" (first render, or no
    // layout engine): draw for the CSS height rather than for no rows at all.
    observeElementRect: (instance, cb) =>
      observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : { width: rect.width, height: WINDOW_PX })),
  })
  return (
    <div ref={scroller} className="h-64 overflow-y-auto" data-testid="windowed-rows">
      <ul className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((row) => {
          const object = objects[row.index]
          if (!object) return null
          return (
            <li
              key={object.name}
              className="absolute left-0 flex w-full items-center gap-1"
              style={{ height: ROW_PX, transform: `translateY(${row.start}px)` }}
            >
              <ObjectRow
                object={object}
                inputType={inputType}
                checked={selected.has(object.name)}
                disabled={disabled}
                onToggle={() => onToggle(object.name, object.label)}
                onColumns={() => onColumns(object.name)}
              />
            </li>
          )
        })}
      </ul>
    </div>
  )
}

/** The listed objects, in full up to `WINDOW_ABOVE` rows and windowed beyond. */
export function ObjectRows(props: RowsProps) {
  return props.objects.length > WINDOW_ABOVE ? <WindowedRows {...props} /> : <PlainRows {...props} />
}

/**
 * The columns of one object (D6). A type a database declares reads as it is; a type guessed
 * from a sample (a CSV's first 1,000 rows, 100 documents or messages, a REST endpoint's first
 * page) is said to be inferred, because it is what the first load will create.
 */
export function ColumnsView({
  name,
  entry,
  onRetry,
  onClose,
}: {
  name: string
  entry: PreviewEntry | undefined
  onRetry: () => void
  onClose: () => void
}) {
  return (
    <div className="space-y-1.5 rounded-md border border-border bg-muted/20 p-2" aria-label={`Columns of ${name}`} role="group">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-mono text-xs">{name}</span>
        <Button type="button" size="xs" variant="ghost" onClick={onClose}>
          Close
        </Button>
      </div>
      {entry === undefined || entry.status === "loading" ? (
        <p className="text-xs text-muted-foreground" role="status">
          Reading the columns…
        </p>
      ) : entry.status === "failed" ? (
        <div className="flex items-center gap-2">
          <p className="text-xs text-muted-foreground">Cannot read the columns · {entry.reason}</p>
          <Button type="button" size="xs" variant="outline" onClick={onRetry}>
            Try again
          </Button>
        </div>
      ) : entry.columns.length === 0 ? (
        <p className="text-xs text-muted-foreground">Nothing to sample yet, so there are no columns to show.</p>
      ) : (
        <>
          <ul className="max-h-48 space-y-0.5 overflow-y-auto">
            {entry.columns.map((column) => (
              <li key={column.name} className="flex items-center gap-2 font-mono text-xs">
                <span className="min-w-0 flex-1 truncate">{column.name}</span>
                <span className="shrink-0 text-muted-foreground">{column.typeName}</span>
                {entry.primaryKey.includes(column.name) ? (
                  <span className="shrink-0 font-sans text-muted-foreground">key</span>
                ) : null}
              </li>
            ))}
          </ul>
          {entry.columns.some((column) => column.inferred) ? (
            <p className="text-xs text-muted-foreground">
              Types are inferred from a sample: they are what the first load will create, and a column that
              appears only later is found by the load.
            </p>
          ) : null}
        </>
      )}
    </div>
  )
}

export type TableFinderProps = {
  connectorId: string
  adapter: string
  dial: Dial
  /** The schema a SQL source lists by default (`defaultDiscoverSchema`). */
  defaultSchema: string
  previews: ColumnPreviews
  selectedNames: ReadonlySet<string>
  /** `label` is the listed object's label, when the vendor gave one (it names the Bronze table by default). */
  onToggle: (name: string, label?: string) => void
  /** Tick (`true`) or untick (`false`) every one of `names`: the buttons over the shown rows. `labels` are the shown objects' labels. */
  onSetMany: (names: string[], on: boolean, labels?: Record<string, string>) => void
  /** Kafka: one topic per connector, so the list is radio buttons and has no "select shown" (D4). */
  single?: boolean
  /** Locks the list while a pick is being saved. */
  busy?: boolean
  /** The connector page's wide layout: inputs do not stretch across the card. */
  wide?: boolean
  /** Poll interval of a running orchestrator listing; tests only. */
  pollMs?: number
}

/** The Find flow itself: inputs for the type, the list with search, and the columns of the opened object. */
export function TableFinder({
  connectorId,
  adapter,
  dial,
  defaultSchema,
  previews,
  selectedNames,
  onToggle,
  onSetMany,
  single = false,
  busy = false,
  wide = false,
  pollMs,
}: TableFinderProps) {
  const kind = finderKind(adapter, dial)
  const words = kind ? FINDER_WORDS[kind] : FINDER_WORDS.sql
  const [schema, setSchema] = React.useState(defaultSchema)
  const [glob, setGlob] = React.useState("")
  const list = useDiscoveryList(connectorId, pollMs)
  const schemaList = useDiscoveryList(connectorId, pollMs)
  const search = useDiscoveryList(connectorId, pollMs)
  // What the last Find asked for: a search narrows that same listing.
  const [asked, setAsked] = React.useState<DiscoveryRequest | null>(null)
  const [filter, setFilter] = React.useState("")
  const [opened, setOpened] = React.useState<string | null>(null)
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  React.useEffect(() => () => clearTimeout(timer.current), [])

  if (kind === null) return null
  const hasSchemas = kind === "sql" || kind === "oracle"
  const pending = list.status === "pending" || schemaList.status === "pending"
  const waitedMs = list.waitedMs ?? schemaList.waitedMs

  const base = list.data?.state === "done" ? list.data : null
  const baseObjects = base?.objects ?? []
  const serverSearch =
    base !== null && (base.truncated === true || (ANSWERS_INLINE.has(kind) && baseObjects.length > SERVER_SEARCH_ABOVE))
  const needle = filter.trim().toLowerCase()
  const searched = serverSearch && needle !== "" && search.data?.state === "done" ? search.data : null
  const objects = searched
    ? (searched.objects ?? [])
    : serverSearch || needle === ""
      ? baseObjects
      : baseObjects.filter((o) => o.name.toLowerCase().includes(needle) || (o.label ?? "").toLowerCase().includes(needle))
  const schemas = (schemaList.data?.state === "done" ? (schemaList.data.objects ?? []) : []).filter(
    (o) => o.kind === "schema"
  )
  const allShownSelected = objects.length > 0 && objects.every((o) => selectedNames.has(o.name))

  function find() {
    const request: DiscoveryRequest = hasSchemas
      ? { schema: schema.trim() }
      : kind === "files" || kind === "sftp"
        ? { glob: glob.trim() }
        : {}
    setAsked(request)
    setFilter("")
    setOpened(null)
    search.reset()
    void list.run(request)
  }

  function onFilter(value: string) {
    setFilter(value)
    clearTimeout(timer.current)
    const term = value.trim()
    if (!serverSearch || asked === null) return
    if (term === "") {
      search.reset()
      return
    }
    // D9: past a page the server narrows the list, once typing pauses.
    timer.current = setTimeout(() => void search.run({ ...asked, search: term }), SEARCH_DEBOUNCE_MS)
  }

  function openColumns(name: string) {
    setOpened(name)
    previews.ensure(name)
  }

  const failed = list.data?.state === "failed" ? list.data : null
  const nothing = base !== null && baseObjects.length === 0
  const inputClass = cn("min-w-0 flex-1 space-y-1", wide && "max-w-xs")

  return (
    <div className="space-y-2 rounded-md border border-border p-3">
      <div className="flex items-end gap-2">
        {hasSchemas ? (
          <div className={inputClass}>
            <Label htmlFor="discover-schema" className="text-xs">
              Schema
            </Label>
            <Input
              id="discover-schema"
              list="discover-schemas"
              value={schema}
              onChange={(e) => setSchema(e.target.value)}
              className="h-8"
              autoComplete="off"
            />
            <datalist id="discover-schemas">
              {schemas.map((s) => (
                <option key={s.name} value={s.name} />
              ))}
            </datalist>
          </div>
        ) : kind === "files" || kind === "sftp" ? (
          <div className={inputClass}>
            <Label htmlFor="discover-glob" className="text-xs">
              File pattern
            </Label>
            <Input
              id="discover-glob"
              value={glob}
              onChange={(e) => setGlob(e.target.value)}
              placeholder="sales/2026-*.csv"
              className="h-8 font-mono text-xs"
              autoComplete="off"
            />
          </div>
        ) : (
          <p className="min-w-0 flex-1 text-xs text-muted-foreground">
            {kind === "rest"
              ? "Lists the endpoints this connector declares."
              : kind === "mongodb"
                ? "Lists the collections of this connector's database."
                : kind === "saas"
                  ? "Lists the objects the vendor offers, and says which load only what changed."
                : "Lists the topics of this connector's broker; internal ones are hidden."}
          </p>
        )}
        {hasSchemas ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={pending || busy}
            onClick={() => void schemaList.run({})}
          >
            List schemas
          </Button>
        ) : null}
        <Button type="button" size="sm" variant="outline" disabled={pending || busy || (hasSchemas && schema.trim() === "")} onClick={find}>
          <SearchIcon data-icon="inline-start" />
          {list.status === "pending" ? "Finding…" : words.find}
        </Button>
      </div>
      {kind === "files" || kind === "sftp" ? (
        <p className="text-xs text-muted-foreground">
          Leave the pattern empty to list every file under this connector&apos;s {kind === "files" ? "prefix" : "path"}.{" "}
          <code>*</code> stays inside a folder, <code>**</code> crosses folders. At most 10,000 files are listed.
        </p>
      ) : null}
      {pending ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground" role="status">
          <LoaderIcon className="size-3.5 animate-spin" />
          <span>
            {waitedMs !== null
              ? `Waiting for the orchestrator, ${secondsOf(waitedMs)} s. This source is listed by a short job, which takes some seconds longer.`
              : ANSWERS_INLINE.has(kind)
                ? "Asking the source…"
                : "Starting the listing…"}
          </span>
          <Button type="button" size="xs" variant="ghost" onClick={() => (list.status === "pending" ? list.cancel() : schemaList.cancel())}>
            Cancel
          </Button>
        </div>
      ) : null}
      {list.status === "error" ? <ErrorState error={list.error} /> : null}
      {schemaList.status === "error" ? <ErrorState error={schemaList.error} /> : null}
      {schemaList.data?.state === "failed" ? (
        <p className="text-xs text-muted-foreground">Cannot list schemas · {failureReason(schemaList.data)}</p>
      ) : null}
      {failed ? <p className="text-xs text-muted-foreground">Cannot list {words.many} · {failureReason(failed)}</p> : null}
      {nothing ? (
        <p className="text-xs text-muted-foreground">
          {hasSchemas
            ? `No tables in schema “${asked?.schema ?? schema.trim()}”. Check the schema name.`
            : kind === "files" || kind === "sftp"
              ? "No files match this pattern."
              : `No ${words.many} found.`}
        </p>
      ) : null}
      {base !== null && baseObjects.length > 0 ? (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            {baseObjects.length > 8 || base.truncated ? (
              <Input
                aria-label={`Filter ${words.many}`}
                value={filter}
                onChange={(e) => onFilter(e.target.value)}
                placeholder={`${serverSearch ? "Search" : "Filter"} ${base.truncated ? "all" : baseObjects.length} ${words.many}`}
                className={cn("h-7 text-xs", wide && "max-w-xs")}
                autoComplete="off"
              />
            ) : (
              <span className="flex-1 text-xs text-muted-foreground">
                {baseObjects.length} {words.many} found
              </span>
            )}
            {single ? null : (
              <Button
                type="button"
                size="xs"
                variant="ghost"
                className="shrink-0"
                onClick={() =>
                  onSetMany(
                    objects.map((o) => o.name),
                    !allShownSelected,
                    Object.fromEntries(objects.flatMap((o) => (o.label ? [[o.name, o.label]] : [])))
                  )
                }
              >
                {allShownSelected ? "Clear shown" : "Select shown"}
              </Button>
            )}
          </div>
          {base.truncated && base.reason ? <p className="text-xs text-muted-foreground">{base.reason}</p> : null}
          {search.status === "pending" ? (
            <p className="text-xs text-muted-foreground" role="status">
              Searching…
            </p>
          ) : null}
          {search.status === "error" ? <ErrorState error={search.error} /> : null}
          {search.data?.state === "failed" && serverSearch && needle !== "" ? (
            <p className="text-xs text-muted-foreground">Cannot search · {failureReason(search.data)}</p>
          ) : null}
          {objects.length === 0 && needle !== "" && search.status !== "pending" ? (
            <p className="text-xs text-muted-foreground">Nothing matches &ldquo;{filter.trim()}&rdquo;.</p>
          ) : null}
          <ObjectRows
            objects={objects}
            inputType={single ? "radio" : "checkbox"}
            selected={selectedNames}
            disabled={busy}
            onToggle={onToggle}
            onColumns={openColumns}
          />
          {opened !== null ? (
            <ColumnsView
              name={opened}
              entry={previews.entries.get(opened)}
              onRetry={() => previews.ensure(opened, true)}
              onClose={() => setOpened(null)}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
