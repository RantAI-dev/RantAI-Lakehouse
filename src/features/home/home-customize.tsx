"use client"

import * as React from "react"
import { GripVertical, Plus, SlidersHorizontal, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Sortable,
  SortableContent,
  SortableItem,
  SortableItemHandle,
} from "@/components/ui/sortable"
import { WIDE_CARD, type CardId, type ShortcutId } from "@/lib/home-layout"
import { SHORTCUTS } from "./home-shortcuts"

/**
 * Edit mode for Home: the pieces that make cards and shortcuts movable and
 * removable, and the bar that saves, cancels or resets.
 *
 * Reordering uses the repo's `Sortable` (dnd-kit): a handle button per item
 * that works with the mouse, touch and the keyboard (space to pick up,
 * arrows to move, space to drop). Hiding is a plain button, so nothing here
 * is reachable only by dragging.
 *
 * While editing, a card's own content is `inert`: its links and buttons do
 * not act, so a click aimed at a handle or at "hide" cannot also navigate.
 */
export const CARD_LABEL: Record<CardId, string> = {
  "dashboard-preview": "Dashboard preview",
  recent: "Recent",
  "pipeline-runs": "Pipeline runs",
  sources: "Sources",
  "open-alerts": "Open alerts",
  "saved-queries": "Saved queries",
}

/** The quiet entry point at the top right of the second screen. */
export function CustomizeButton({ onClick }: { onClick: () => void }) {
  return (
    <Button variant="ghost" size="sm" onClick={onClick} className="text-muted-foreground">
      <SlidersHorizontal />
      Customize
    </Button>
  )
}

/** With every card hidden, the page says so instead of going blank. */
export function EmptyHome({
  onCustomize,
  message = "No cards are shown on Home.",
}: {
  onCustomize?: () => void
  message?: string
}) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border px-4 py-10 text-center">
      <p className="text-sm text-muted-foreground">{message}</p>
      {onCustomize ? (
        <Button variant="outline" size="sm" onClick={onCustomize}>
          <SlidersHorizontal />
          Customize
        </Button>
      ) : null}
    </div>
  )
}

type AddMenuProps<T extends string> = {
  label: string
  options: T[]
  nameOf: (id: T) => string
  onAdd: (id: T) => void
  /** Render the trigger as an empty slot in a row instead of a button. */
  slot?: boolean
}

function AddMenu<T extends string>({ label, options, nameOf, onAdd, slot }: AddMenuProps<T>) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          slot ? (
            <button
              type="button"
              className="flex min-w-0 items-center justify-center gap-2 rounded-xl border border-dashed border-border px-3 py-2.5 text-sm font-medium text-muted-foreground transition-colors outline-none hover:border-foreground/30 hover:bg-muted/50 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              <Plus className="size-4" />
              {label}
            </button>
          ) : (
            <Button variant="outline" size="sm" disabled={options.length === 0}>
              <Plus />
              {label}
            </Button>
          )
        }
      />
      <DropdownMenuContent align={slot ? "start" : "end"}>
        {options.map((id) => (
          <DropdownMenuItem key={id} onClick={() => onAdd(id)}>
            {nameOf(id)}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * Save, discard or reset. It sticks to the bottom of the scroller, so it is
 * in reach from either screen: the shortcuts being edited are on the first,
 * the cards on the second.
 */
export function EditBar({
  hiddenCards,
  busy,
  error,
  onAddCard,
  onReset,
  onCancel,
  onDone,
}: {
  hiddenCards: CardId[]
  busy: boolean
  error: string | null
  onAddCard: (id: CardId) => void
  onReset: () => void
  onCancel: () => void
  onDone: () => void
}) {
  return (
    <div
      role="region"
      aria-label="Customize Home"
      className="sticky bottom-4 z-20 mt-6 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-border bg-popover px-3 py-2 text-popover-foreground shadow-lg"
    >
      <p className="min-w-0 flex-1 text-xs text-muted-foreground">
        Drag the handles to reorder, or hide what you don&apos;t need. Your choice is saved to your account.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <AddMenu
          label="Add card"
          options={hiddenCards}
          nameOf={(id) => CARD_LABEL[id]}
          onAdd={onAddCard}
        />
        <Button variant="ghost" size="sm" onClick={onReset} disabled={busy}>
          Reset to default
        </Button>
        <Button variant="outline" size="sm" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button size="sm" onClick={onDone} disabled={busy}>
          Done
        </Button>
      </div>
      {error ? (
        <p role="alert" className="basis-full text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  )
}

const HANDLE_CLASS =
  "inline-flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"

function HideButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <Button variant="ghost" size="icon-xs" aria-label={`Hide ${label}`} title="Hide" onClick={onClick}>
      <X />
    </Button>
  )
}

function FrameBar({
  label,
  onHide,
  handle,
}: {
  label: string
  onHide: () => void
  handle?: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-1 pb-2">
      {handle}
      <span className="min-w-0 flex-1 truncate px-1 text-xs font-medium text-muted-foreground">
        {label}
      </span>
      <HideButton label={label} onClick={onHide} />
    </div>
  )
}

const FRAME_CLASS = "min-w-0 rounded-xl border border-dashed border-border p-2"

function Inert({ children }: { children: React.ReactNode }) {
  return (
    <div inert className="pointer-events-none select-none">
      {children}
    </div>
  )
}

function SortableCardFrame({
  id,
  onHide,
  children,
}: {
  id: CardId
  onHide: () => void
  children: React.ReactNode
}) {
  const label = CARD_LABEL[id]
  return (
    <SortableItem value={id} className={FRAME_CLASS}>
      <FrameBar
        label={label}
        onHide={onHide}
        handle={
          <SortableItemHandle aria-label={`Move ${label}`} className={HANDLE_CLASS}>
            <GripVertical className="size-4" />
          </SortableItemHandle>
        }
      />
      <Inert>{children}</Inert>
    </SortableItem>
  )
}

/** The grid for cards that are not beside the wide one. */
export function gridClass(n: number): string {
  const cols =
    n <= 1 ? "" : n === 2 ? "lg:grid-cols-2" : "md:grid-cols-2 lg:grid-cols-3"
  return `grid min-w-0 grid-cols-1 items-start gap-6 ${cols}`
}

/**
 * Screen 2 while editing. The same two shapes as outside edit mode: the
 * wide card (the dashboard preview) in its own column when it is shown, the
 * others stacked beside it; otherwise a grid. The wide card has no drag
 * handle because the layout fixes it to the wide column wherever it sits in
 * the order, so dragging it would promise a move that never happens.
 */
export function EditableCards({
  cards,
  nodes,
  previewPicker,
  onReorder,
  onHide,
}: {
  cards: CardId[]
  nodes: Record<CardId, React.ReactNode>
  previewPicker: React.ReactNode
  onReorder: (others: CardId[]) => void
  onHide: (id: CardId) => void
}) {
  const wide = cards.includes(WIDE_CARD)
  const others = cards.filter((c) => c !== WIDE_CARD)
  const items = others.map((id) => (
    <SortableCardFrame key={id} id={id} onHide={() => onHide(id)}>
      {nodes[id]}
    </SortableCardFrame>
  ))

  if (cards.length === 0) {
    return <EmptyHome />
  }
  if (wide) {
    return (
      <Sortable value={others} onValueChange={onReorder} orientation="vertical">
        <div className="grid min-w-0 grid-cols-1 items-start gap-6 lg:grid-cols-3">
          <div className="min-w-0 lg:col-span-2">
            <div className={FRAME_CLASS}>
              <FrameBar label={CARD_LABEL[WIDE_CARD]} onHide={() => onHide(WIDE_CARD)} />
              <div className="flex flex-wrap items-center gap-2 px-1 pb-3">{previewPicker}</div>
              <Inert>{nodes[WIDE_CARD]}</Inert>
            </div>
          </div>
          <SortableContent className="flex min-w-0 flex-col gap-6">{items}</SortableContent>
        </div>
      </Sortable>
    )
  }
  return (
    <Sortable value={others} onValueChange={onReorder} orientation="mixed">
      <SortableContent className={gridClass(others.length)}>{items}</SortableContent>
    </Sortable>
  )
}

/** The shortcut row while editing: the same three-up grid, each one a movable chip. */
export function EditableShortcuts({
  ids,
  hidden,
  onReorder,
  onHide,
  onAdd,
}: {
  ids: ShortcutId[]
  hidden: ShortcutId[]
  onReorder: (next: ShortcutId[]) => void
  onHide: (id: ShortcutId) => void
  onAdd: (id: ShortcutId) => void
}) {
  return (
    <Sortable value={ids} onValueChange={onReorder} orientation="mixed">
      <SortableContent className="grid gap-2 sm:grid-cols-3">
        {ids.map((id) => {
          const { icon: Icon, title } = SHORTCUTS[id]
          return (
            <SortableItem
              key={id}
              value={id}
              className="flex min-w-0 items-center gap-1 rounded-xl border border-dashed border-border bg-background px-2 py-2.5"
            >
              <SortableItemHandle aria-label={`Move ${title}`} className={HANDLE_CLASS}>
                <GripVertical className="size-4" />
              </SortableItemHandle>
              <Icon className="size-4 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{title}</span>
              <HideButton label={title} onClick={() => onHide(id)} />
            </SortableItem>
          )
        })}
        {/* The free slot is the add button, in the row where the new
            shortcut will land. A small button under the row, disabled
            while the row was full, was missed (QA feedback). */}
        {ids.length < 3 && hidden.length > 0 ? (
          <AddMenu
            slot
            label="Add shortcut"
            options={hidden}
            nameOf={(id) => SHORTCUTS[id].title}
            onAdd={onAdd}
          />
        ) : null}
      </SortableContent>
    </Sortable>
  )
}

const LAST_OPENED = "__last-opened"

/**
 * Which dashboard the preview card shows: the one last opened in this
 * browser (the default), or one chosen by name. A board with no charts can
 * be picked but cannot be previewed, so the label says it.
 */
export function PreviewPicker({
  boards,
  value,
  onChange,
}: {
  boards: { id: string; name: string; chartCount?: number }[] | null
  value: string | null
  onChange: (id: string | null) => void
}) {
  const chosen = boards?.find((b) => b.id === value)
  return (
    <>
      <span className="text-xs text-muted-foreground">Dashboard</span>
      <Select
        value={chosen ? chosen.id : LAST_OPENED}
        onValueChange={(v) => onChange(!v || v === LAST_OPENED ? null : v)}
        disabled={boards === null}
      >
        <SelectTrigger size="sm" aria-label="Dashboard to preview">
          <SelectValue>{chosen ? chosen.name : "Last opened"}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={LAST_OPENED}>Last opened</SelectItem>
          {(boards ?? []).map((b) => (
            <SelectItem key={b.id} value={b.id}>
              {b.name}
              {(b.chartCount ?? 0) === 0 ? " (no charts)" : ""}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </>
  )
}
