"use client"

/**
 * The meaning of a table and of its columns on the Schema tab: the table's
 * card above the Columns card, and the Meaning view that the Columns card
 * switches to. The pure rules (which keys have an entry, how entries split,
 * how a save body is built) are in `asset-meaning.ts`.
 *
 * A person writes here only through the state's `save`, with a body built by
 * `inputOf`, so no save drops a field the API replaces.
 */
import * as React from "react"
import { Check } from "lucide-react"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { formatRelativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SemanticEntry, SemanticEntryInput } from "@/services/contracts/semantic"
import type { SchemaRow } from "./asset-columns"
import { inputOf, parseSynonyms, ROLE_OPTIONS, roleLabel, type MeaningState } from "./asset-meaning"

type Ready = Extract<MeaningState, { kind: "ready" }>

/** The API's limits on a description (`semantic_api.rs` `TABLE_TEXT_CHARS`, `COLUMN_TEXT_CHARS`). */
const TABLE_TEXT_MAX = 400
const COLUMN_TEXT_MAX = 200

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
  if (state.kind === "ready") return <TableMeaningReady state={state} />
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
  return null
}

/**
 * The table's card once its meanings are loaded. The hooks live here, not in
 * `TableMeaningCard`, so the card can return early for the other states.
 */
function TableMeaningReady({ state }: { state: Ready }) {
  const { table, canEdit } = state
  const [editing, setEditing] = React.useState(false)
  const confirm = useServiceAction((signal, input: SemanticEntryInput) => state.save(input, signal))
  const draft = table?.status === "draft"
  const busy = confirm.status === "pending"

  async function confirmTable() {
    if (!table) return
    const ok = await confirm.run(inputOf(table))
    // A null result is a failure, which the card shows, or a save a later one aborted.
    if (ok !== null) state.reload()
  }

  return (
    <SectionCard
      size="sm"
      title="Meaning"
      description={TABLE_DESCRIPTION}
      action={
        canEdit ? (
          <span className="flex items-center gap-1.5">
            {confirm.status === "error" ? (
              <span role="alert" className="text-xs text-destructive">
                {confirm.error.message}
              </span>
            ) : null}
            {draft ? (
              <Button size="sm" variant="outline" disabled={busy} onClick={() => void confirmTable()}>
                Confirm
              </Button>
            ) : null}
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => setEditing(true)}>
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
      {editing ? (
        // Mounted only while open, so each opening starts from the saved values.
        <MeaningDialog state={state} target={{ column: null, entry: table }} onClose={() => setEditing(false)} />
      ) : null}
    </SectionCard>
  )
}

// ── The Meaning view ──────────────────────────────────────────────────────
//
// Header and rows share one grid string, as the Columns card does: fixed
// tracks in `rem`, flexible ones `minmax(0, …fr)`, so a row truncates and
// never widens the list. Cells shed by container query, in this order
// (PR #109 review, finding 4): the synonyms go first, below `@3xl`; the status
// goes second, below `@2xl`, so a reader who cannot write still sees whether
// an entry is a draft. The column name, the role, the meaning and the confirm
// control never shed. Every breakpoint lists the cells that are shown, in
// order, so the header and the rows agree. Below `@lg` the four shown cells
// take one track each, so a row is one line (plan 10 review 2): the meaning
// gets the widest track (2fr) of the three text cells, since it is what the
// row is for, and the confirm control keeps its fixed 1.75rem.
const MEANING_ROW = "grid items-center gap-x-3 px-3"
const MEANING_TRACKS = [
  "grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,2fr)_1.75rem]",
  "@lg:grid-cols-[minmax(0,1fr)_9rem_minmax(0,2fr)_1.75rem]",
  "@2xl:grid-cols-[minmax(0,1fr)_9rem_minmax(0,2fr)_5rem_1.75rem]",
  "@3xl:grid-cols-[minmax(0,1fr)_9rem_minmax(0,2fr)_minmax(0,1fr)_5rem_1.75rem]",
].join(" ")

/**
 * The role choice shared by a row and the dialog. `NOT_SET` is the "Not set"
 * item: base-ui does not report an empty-string item to `onValueChange`, so the
 * item carries a sentinel. It stays private to this component: `null` goes in
 * and comes out, and the caller never sees the sentinel. The value shown
 * is the entry's own, so a failed save shows the stored role again.
 * PR #109 review, finding 3.
 */
const NOT_SET = "none"

function RoleSelect({
  value,
  onChange,
  label,
  disabled,
  size = "sm",
}: {
  /** The stored role, or `null` for "Not set". */
  value: string | null
  onChange: (role: string | null) => void
  label: string
  disabled?: boolean
  size?: "sm" | "default"
}) {
  return (
    <Select
      value={value ?? NOT_SET}
      onValueChange={(v) => onChange(v === NOT_SET || v === null ? null : v)}
      disabled={disabled}
    >
      <SelectTrigger size={size} aria-label={label} className="w-full">
        <SelectValue>{roleLabel(value)}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={NOT_SET}>Not set</SelectItem>
        {ROLE_OPTIONS.map((o) => (
          <SelectItem key={o.value} value={o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

/** One row of the Meaning view: a column of the table, with its entry when it has one. */
function MeaningRow({
  row,
  entry,
  canEdit,
  busy,
  onRole,
  onConfirm,
  onEdit,
}: {
  row: SchemaRow
  entry: SemanticEntry | undefined
  canEdit: boolean
  /** A save is running: every control of the row is off, since a second save would abort the first. */
  busy: boolean
  onRole: (role: string | null) => void
  onConfirm: () => void
  onEdit: () => void
}) {
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
          <RoleSelect
            value={role}
            onChange={onRole}
            label={`Role of ${name}`}
            disabled={!entry || busy}
          />
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
              disabled={busy}
              onClick={onEdit}
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
            disabled={busy}
            onClick={onEdit}
            className="h-auto justify-start px-1 py-0.5 text-xs font-normal text-muted-foreground"
          >
            Add a meaning
          </Button>
        ) : (
          <span className="block truncate text-xs text-muted-foreground">No meaning written</span>
        )}
      </div>
      <div role="cell" className="hidden min-w-0 @3xl:block">
        <span className="block truncate text-xs text-muted-foreground" title={entry?.synonyms.join(", ")}>
          {entry?.synonyms.join(", ") ?? ""}
        </span>
      </div>
      <div role="cell" className="hidden @2xl:block">
        {entry ? <Pill tone={draft ? "warning" : "neutral"}>{entry.status}</Pill> : null}
      </div>
      <div role="cell" className="flex justify-end">
        {canEdit && draft ? (
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={busy}
            onClick={onConfirm}
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
      <div role="columnheader" className="hidden @3xl:block">
        Also called
      </div>
      <div role="columnheader" className="hidden @2xl:block">
        Status
      </div>
      <div role="columnheader">
        <span className="sr-only">Confirm</span>
      </div>
    </div>
  )
}

/** The entry a dialog starts from: the stored one, or a blank for a column or table without one. */
function startingEntry(column: string | null, entry: SemanticEntry | null): SemanticEntry {
  // Only the fields `inputOf` reads matter; a blank is never sent as it is, since the dialog overwrites its text.
  return (
    entry ?? {
      asset: "",
      column,
      description: "",
      synonyms: [],
      role: null,
      status: "draft",
      writtenBy: null,
      model: null,
      updatedAt: "",
    }
  )
}

type DialogTarget = { column: string | null; entry: SemanticEntry | null }

/**
 * Edits one meaning: the text, the synonyms and, for a column, the role. Built
 * like `EditDialog` in `asset-about.tsx`, and mounted only while open.
 */
function MeaningDialog({ state, target, onClose }: { state: Ready; target: DialogTarget; onClose: () => void }) {
  const id = React.useId()
  const { column } = target
  const base = startingEntry(column, target.entry)
  const max = column === null ? TABLE_TEXT_MAX : COLUMN_TEXT_MAX
  const [description, setDescription] = React.useState(base.description)
  const [synonyms, setSynonyms] = React.useState(base.synonyms.join(", "))
  const [role, setRole] = React.useState<string | null>(base.role)
  const save = useServiceAction((signal, input: SemanticEntryInput) => state.save(input, signal))
  const parsed = parseSynonyms(synonyms)
  const text = description.trim()
  // The API counts code points, so a count by `length` would disagree for a character outside the BMP.
  const length = [...description].length
  const tooLong = length > max

  const title = column === null ? "Edit the table's meaning" : `Edit the meaning of ${column}`
  const invalid = text === "" || tooLong

  async function submit() {
    if (invalid || "error" in parsed) return
    const input = inputOf(base, {
      description: text,
      synonyms: parsed.synonyms,
      // The table has no role at all; the API refuses one on a table.
      role: column === null ? null : role,
    })
    const ok = await save.run(input)
    // `run` resolves to `null` only on failure, which `save.error` shows.
    if (ok === null) return
    notifySuccess("Meaning saved")
    state.reload()
    onClose()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Saving marks this as confirmed by a person. The assistant uses it from its next answer.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor={`${id}-meaning`}>Meaning</Label>
              <span className={cn("text-xs", tooLong ? "text-destructive" : "text-muted-foreground")}>
                {`${length}/${max}`}
              </span>
            </div>
            <Textarea
              id={`${id}-meaning`}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              aria-invalid={text === "" || tooLong}
              rows={4}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor={`${id}-synonyms`}>Also called</Label>
            <Input
              id={`${id}-synonyms`}
              value={synonyms}
              onChange={(e) => setSynonyms(e.target.value)}
              placeholder="revenue, sales, gross"
              aria-invalid={"error" in parsed}
            />
            <p className={cn("text-xs", "error" in parsed ? "text-destructive" : "text-muted-foreground")}>
              {"error" in parsed ? parsed.error : "Separate with commas. Up to 6 names."}
            </p>
          </div>
          {column !== null ? (
            <div className="grid gap-1.5">
              <Label>Role</Label>
              <RoleSelect value={role} onChange={setRole} label="Role" disabled={save.status === "pending"} size="default" />
              {ROLE_OPTIONS.find((o) => o.value === role) ? (
                <p className="text-xs text-muted-foreground">
                  {ROLE_OPTIONS.find((o) => o.value === role)?.help}
                </p>
              ) : null}
            </div>
          ) : null}
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={invalid || "error" in parsed || save.status === "pending"}
          >
            {save.status === "pending" ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * The Meaning view: one line per listed column, with its role, its meaning,
 * its synonyms and its status. A column without an entry says so in place.
 * A role is saved from its row at once; a meaning is edited in the dialog.
 */
export function MeaningView({ rows, state }: { rows: SchemaRow[]; state: Ready }) {
  const { byColumn, canEdit, save, reload } = state
  const [editing, setEditing] = React.useState<DialogTarget | null>(null)
  // One action for the whole view: `run` aborts the save before it, so the controls are off while one runs.
  const write = useServiceAction((signal, input: SemanticEntryInput) => save(input, signal))
  // The column a save is for, so a failure can name it. Set before each run, never reset.
  const [target, setTarget] = React.useState<string | null>(null)
  const busy = write.status === "pending"

  async function saveRow(name: string, input: SemanticEntryInput) {
    setTarget(name)
    const ok = await write.run(input)
    // A null result is a failure, which the alert names, or a save that a later one aborted.
    if (ok !== null) reload()
  }

  return (
    <>
      <div className="@container overflow-hidden rounded-lg border border-border">
        <div role="table" aria-label="Column meanings">
          <div role="rowgroup">
            <MeaningHeader />
          </div>
          <div role="rowgroup">
            {rows.map((row) => {
              const name = row.column.name
              const entry = byColumn.get(name)
              return (
                <MeaningRow
                  key={name}
                  row={row}
                  entry={entry}
                  canEdit={canEdit}
                  busy={busy}
                  onRole={(v) => {
                    if (entry) void saveRow(name, inputOf(entry, { role: v }))
                  }}
                  onConfirm={() => {
                    if (entry) void saveRow(name, inputOf(entry))
                  }}
                  onEdit={() => setEditing({ column: name, entry: entry ?? null })}
                />
              )
            })}
          </div>
        </div>
      </div>
      {write.status === "error" && target !== null ? (
        <p role="alert" className="text-sm text-destructive">
          {`Could not save ${target}: ${write.error.message}`}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">
        Saving a row marks it as confirmed by a person. The assistant uses it from its next answer.
      </p>
      {editing ? (
        // Mounted only while open, so each opening starts from the saved values.
        <MeaningDialog state={state} target={editing} onClose={() => setEditing(null)} />
      ) : null}
    </>
  )
}
