"use client"

/**
 * The report editor's floating parts (`BI-62`, mockup D): the `/` insert
 * menu, the selection toolbar (bold, italic, link: links live inside
 * sentences now, replacing the Link block), and the ⋮⋮ / + handles beside each
 * block. Written here, not taken from a component kit (no restrictively
 * licensed code in this AGPL repository).
 *
 * Motion is short and eased (150 to 200 ms, none when reduced motion is
 * asked), and every piece answers the keyboard: ↑ ↓ Enter Esc in the menu,
 * ⌘B ⌘I ⌘K for marks, Alt+↑ ↓ to move a block.
 */
import * as React from "react"
import { createPortal } from "react-dom"
import type { Editor } from "@tiptap/core"
import { NodeSelection } from "@tiptap/pm/state"
import { useEditorState } from "@tiptap/react"
import { BubbleMenu } from "@tiptap/react/menus"
import { Bold, Check, GripVertical, Italic, Link2, Plus, Unlink } from "lucide-react"

import { safeReportUrl } from "@/lib/report-blocks"
import { CHART_NODE, SLASH_GROUPS } from "@/lib/report-doc"
import { cn } from "@/lib/utils"
import { endBlockDrag, startBlockDrag, type SlashItem, type SlashStore } from "./report-editor-extensions"
import { EASE, FOCUS, IconTip } from "./report-editor-ui"

/* ------------------------------------------------------------------ */
/* The "/" menu                                                        */
/* ------------------------------------------------------------------ */

export type SlashIcon = React.ComponentType<{ className?: string }>

const MENU_MAX = 392
const ROW = 36
const GROUP_HEAD = 26

export function SlashMenu({ store, icons }: { readonly store: SlashStore; readonly icons: Readonly<Record<string, SlashIcon>> }) {
  const state = React.useSyncExternalStore(store.subscribe, store.get, store.get)
  const [active, setActive] = React.useState(0)
  const [fade, setFade] = React.useState({ top: false, bottom: false })
  const list = React.useRef<HTMLDivElement>(null)
  const items = state.items
  const groups = SLASH_GROUPS.map((g) => ({ g, rows: items.filter((i) => i.group === g) })).filter((x) => x.rows.length)
  // ↑ ↓ follow the order on screen, skipping what cannot be picked.
  const enabled = groups.flatMap((x) => x.rows).filter((i) => !i.disabled)

  React.useEffect(() => {
    setActive(0)
  }, [state.query, state.open])

  React.useEffect(() => {
    store.setKeyHandler((e) => {
      if (!enabled.length) return false
      if (e.key === "ArrowDown") {
        setActive((a) => (a + 1) % enabled.length)
        return true
      }
      if (e.key === "ArrowUp") {
        setActive((a) => (a - 1 + enabled.length) % enabled.length)
        return true
      }
      if (e.key === "Enter" || e.key === "Tab") {
        state.command?.(enabled[active] ?? enabled[0])
        return true
      }
      return false
    })
    return () => store.setKeyHandler(null)
  })

  // The edges fade only where rows are hidden (DESIGN.md §3).
  const measure = React.useCallback(() => {
    const el = list.current
    if (!el) return
    const top = el.scrollTop > 2
    const bottom = el.scrollTop + el.clientHeight < el.scrollHeight - 2
    setFade((f) => (f.top === top && f.bottom === bottom ? f : { top, bottom }))
  }, [])
  React.useLayoutEffect(() => {
    measure()
  }, [measure, items.length, state.open])

  React.useEffect(() => {
    list.current?.querySelector<HTMLElement>("[data-active=true]")?.scrollIntoView?.({ block: "nearest" })
  }, [active])

  if (!state.open || !state.rect) return null
  const r = state.rect
  // Below the line when it fits; above it only when below does not, and never over the line itself.
  const natural = Math.min(MENU_MAX, Math.max(1, items.length) * ROW + groups.length * GROUP_HEAD + 8)
  const spaceBelow = window.innerHeight - r.bottom - 6 - 8
  const spaceAbove = r.top - 6 - 8
  const below = spaceBelow >= natural || spaceBelow >= spaceAbove
  const maxHeight = Math.min(MENU_MAX, below ? spaceBelow : spaceAbove)
  const left = Math.max(8, Math.min(r.left - 4, window.innerWidth - 280 - 8))
  const mask =
    fade.top || fade.bottom
      ? `linear-gradient(to bottom, ${fade.top ? "transparent, black 20px" : "black"}, ${fade.bottom ? "black calc(100% - 20px), transparent" : "black"})`
      : undefined
  const style: React.CSSProperties = { ...(below ? { top: r.bottom + 6 } : { bottom: window.innerHeight - r.top + 6 }), left, maxHeight }

  // Portalled to <body>: the page is a size container, which would otherwise become the menu's fixed frame.
  return createPortal(
    <div
      style={style}
      className={cn(
        "rd-pop fixed z-50 flex w-[280px] flex-col overflow-hidden rounded-xl bg-popover text-popover-foreground shadow-[0_16px_40px_-12px_rgb(15_23_42/0.28)] ring-1 ring-foreground/8 dark:shadow-[0_16px_40px_-12px_rgb(0_0_0/0.6)] dark:ring-white/10",
        below ? "origin-top-left" : "origin-bottom-left",
      )}
    >
      <div
        role="listbox"
        aria-label="Insert"
        ref={list}
        onScroll={measure}
        style={{ maskImage: mask, WebkitMaskImage: mask }}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-1"
      >
        {items.length === 0 ? (
          <p className="px-2.5 py-2 text-sm text-muted-foreground">Nothing matches “{state.query}”</p>
        ) : (
          groups.map(({ g, rows }) => (
            <div key={g} role="group" aria-label={g} className="py-0.5">
              <p aria-hidden className="px-2 pt-1 pb-0.5 text-xs leading-5 text-muted-foreground">
                {g}
              </p>
              {rows.map((item) => {
                const idx = enabled.indexOf(item)
                const on = idx === active
                const Icon = icons[item.id]
                return (
                  <div
                    key={item.id}
                    role="option"
                    aria-selected={on}
                    aria-disabled={item.disabled || undefined}
                    data-active={on}
                    onMouseMove={() => {
                      if (idx >= 0 && !on) setActive(idx)
                    }}
                    onMouseDown={(e) => {
                      e.preventDefault()
                      if (!item.disabled) state.command?.(item)
                    }}
                    className={cn(
                      "flex h-9 cursor-default items-center gap-2.5 rounded-lg px-1.5 text-sm",
                      on ? "bg-muted text-foreground" : "text-foreground/90",
                      item.disabled && "opacity-50",
                    )}
                  >
                    <span
                      className={cn(
                        "grid size-6.5 shrink-0 place-items-center rounded-md bg-muted/70 text-muted-foreground dark:bg-muted/50",
                        on && "bg-background text-foreground dark:bg-background/60",
                        item.group === "Write with AI" && "bg-sky-500/12 text-sky-700 dark:text-sky-300",
                      )}
                    >
                      {Icon ? <Icon className="size-4" /> : null}
                    </span>
                    <span className="min-w-0 flex-1 truncate">{item.label}</span>
                    {item.hint ? <span className="shrink-0 pr-1 text-xs text-muted-foreground">{item.hint}</span> : null}
                  </div>
                )
              })}
            </div>
          ))
        )}
      </div>
    </div>,
    document.body,
  )
}

/* ------------------------------------------------------------------ */
/* The selection toolbar                                               */
/* ------------------------------------------------------------------ */

/** A bare "example.com" becomes https://example.com; anything else is checked as typed. */
export function normaliseLink(url: string): string {
  const u = url.trim()
  if (/^[a-z0-9-]+(\.[a-z0-9-]+)+(\/.*)?$/i.test(u)) return `https://${u}`
  return u
}

export type LinkOpener = { open: () => void }

export function SelectionToolbar({ editor, linkRef }: { readonly editor: Editor; readonly linkRef: React.RefObject<LinkOpener | null> }) {
  const marks = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      link: e.isActive("link"),
    }),
  })
  const [editing, setEditing] = React.useState(false)
  const [url, setUrl] = React.useState("")
  const [bad, setBad] = React.useState(false)
  const input = React.useRef<HTMLInputElement>(null)

  const openLink = React.useCallback(() => {
    setUrl((editor.getAttributes("link").href as string | undefined) ?? "")
    setBad(false)
    setEditing(true)
    window.requestAnimationFrame(() => input.current?.focus())
  }, [editor])

  React.useImperativeHandle(linkRef, () => ({ open: openLink }), [openLink])

  const apply = () => {
    const href = normaliseLink(url)
    if (!href) {
      editor.chain().focus().extendMarkRange("link").unsetLink().run()
      setEditing(false)
      return
    }
    // The server's rule (plan D3): only the web, an email or a page of the console.
    if (!safeReportUrl(href)) {
      setBad(true)
      return
    }
    editor.chain().focus().extendMarkRange("link").setLink({ href }).run()
    setEditing(false)
  }

  const btn = (on: boolean) =>
    cn(
      "grid size-8 place-items-center rounded-lg active:scale-[0.96]",
      on ? "bg-primary/10 text-primary dark:bg-sky-400/15 dark:text-sky-300" : "text-foreground/80 hover:bg-muted hover:text-foreground",
      EASE,
      FOCUS,
    )

  return (
    <BubbleMenu
      editor={editor}
      options={{ placement: "top", offset: 8, onHide: () => setEditing(false) }}
      shouldShow={({ editor: e, state, from, to }) => {
        if (!e.isEditable || from === to || state.selection instanceof NodeSelection) return false
        return state.selection.$from.parent.type.name !== "title"
      }}
      className="rd-pop z-40 flex items-center gap-0.5 rounded-xl bg-popover p-1 text-popover-foreground shadow-[0_12px_32px_-12px_rgb(15_23_42/0.3)] ring-1 ring-foreground/8 dark:ring-white/10"
    >
      {editing ? (
        <form
          className="flex items-center gap-1"
          onSubmit={(e) => {
            e.preventDefault()
            apply()
          }}
        >
          <label htmlFor="rd-link-url" className="sr-only">
            Link address
          </label>
          <div className="grid">
            <input
              id="rd-link-url"
              ref={input}
              value={url}
              onChange={(e) => {
                setUrl(e.target.value)
                setBad(false)
              }}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault()
                  setEditing(false)
                  editor.commands.focus()
                }
              }}
              placeholder="Paste a link, or /dashboards/…"
              aria-invalid={bad || undefined}
              className={cn(
                "h-8 w-[min(260px,60vw)] rounded-lg bg-muted/70 px-2.5 text-sm outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-ring/40 dark:bg-input/30",
                bad && "ring-2 ring-destructive/50",
              )}
            />
            {bad ? (
              <span role="alert" className="px-1 pt-1 text-[11px] text-destructive">
                Use a web or email address, or a page of the console
              </span>
            ) : null}
          </div>
          <IconTip label="Apply (Enter)">
            <button type="submit" aria-label="Apply link" className={btn(false)}>
              <Check className="size-4" aria-hidden />
            </button>
          </IconTip>
          {marks.link ? (
            <IconTip label="Remove link">
              <button
                type="button"
                aria-label="Remove link"
                className={btn(false)}
                onClick={() => {
                  editor.chain().focus().extendMarkRange("link").unsetLink().run()
                  setEditing(false)
                }}
              >
                <Unlink className="size-4" aria-hidden />
              </button>
            </IconTip>
          ) : null}
        </form>
      ) : (
        <>
          <IconTip label="Bold ⌘B">
            <button type="button" aria-label="Bold" aria-pressed={marks.bold} className={btn(marks.bold)} onClick={() => editor.chain().focus().toggleBold().run()}>
              <Bold className="size-4" aria-hidden />
            </button>
          </IconTip>
          <IconTip label="Italic ⌘I">
            <button type="button" aria-label="Italic" aria-pressed={marks.italic} className={btn(marks.italic)} onClick={() => editor.chain().focus().toggleItalic().run()}>
              <Italic className="size-4" aria-hidden />
            </button>
          </IconTip>
          <span aria-hidden className="mx-0.5 h-4 w-px bg-border" />
          <IconTip label="⌘K">
            <button
              type="button"
              aria-pressed={marks.link}
              onClick={openLink}
              className={cn(
                "inline-flex h-8 items-center gap-1.5 rounded-lg px-2 text-sm active:scale-[0.97]",
                marks.link ? "bg-primary/10 text-primary dark:bg-sky-400/15 dark:text-sky-300" : "text-foreground/80 hover:bg-muted hover:text-foreground",
                EASE,
                FOCUS,
              )}
            >
              <Link2 className="size-4" aria-hidden /> Link
            </button>
          </IconTip>
        </>
      )}
    </BubbleMenu>
  )
}

/* ------------------------------------------------------------------ */
/* Block handles                                                       */
/* ------------------------------------------------------------------ */

type Hovered = { index: number; start: number; top: number; left: number }

/**
 * ⋮⋮ and + beside the block under the pointer. The handle follows the row
 * the pointer is on anywhere across the page, so it can be reached from the
 * margin; it hides while typing and on touch screens, where Alt+↑ ↓ and the
 * `/` menu do the same jobs.
 */
export function BlockHandles({
  editor,
  area,
  wrap,
}: {
  readonly editor: Editor
  readonly area: React.RefObject<HTMLElement | null>
  readonly wrap: React.RefObject<HTMLDivElement | null>
}) {
  const [hovered, setHovered] = React.useState<Hovered | null>(null)
  const [dragging, setDragging] = React.useState(false)

  React.useEffect(() => {
    const el = area.current
    if (!el) return
    const find = (y: number): Hovered | null => {
      const box = wrap.current?.getBoundingClientRect()
      if (!box || !editor.isEditable) return null
      const { doc } = editor.state
      let start = 0
      for (let i = 0; i < doc.childCount; i++) {
        const node = doc.child(i)
        if (i > 0) {
          const dom = editor.view.nodeDOM(start) as HTMLElement | null
          if (dom instanceof HTMLElement) {
            // A chart's view wraps its card; a wide card reaches past the wrapper, so measure the card.
            const r = (dom.querySelector(".rd-chart-block") ?? dom).getBoundingClientRect()
            if (y >= r.top - 10 && y <= r.bottom + 10) {
              const line = node.isTextblock ? parseFloat(getComputedStyle(dom).lineHeight) || 24 : 0
              const first = node.isTextblock ? r.top + line / 2 - 12 : r.top + 10
              const listLike = node.type.name === "bulletList" || node.type.name === "orderedList" || node.type.name === "blockquote"
              const top = listLike ? r.top + 14 - 12 : first
              return { index: i, start, top: top - box.top, left: Math.max(r.left - box.left - 52, -box.left + 4) }
            }
          }
        }
        start += node.nodeSize
      }
      return null
    }
    const onMove = (e: MouseEvent) => {
      if (!dragging) setHovered(find(e.clientY))
    }
    const hide = () => {
      if (!dragging) setHovered(null)
    }
    el.addEventListener("mousemove", onMove)
    el.addEventListener("mouseleave", hide)
    window.addEventListener("scroll", hide, { passive: true })
    editor.view.dom.addEventListener("keydown", hide)
    return () => {
      el.removeEventListener("mousemove", onMove)
      el.removeEventListener("mouseleave", hide)
      window.removeEventListener("scroll", hide)
      editor.view.dom.removeEventListener("keydown", hide)
    }
  }, [editor, area, wrap, dragging])

  if (!hovered) return null
  const node = editor.state.doc.maybeChild(hovered.index)
  if (!node) return null

  const insertBelow = () => {
    const after = hovered.start + node.nodeSize
    if (node.type.name === "paragraph" && node.content.size === 0) {
      editor.chain().focus().setTextSelection(hovered.start + 1).insertContent("/").run()
    } else {
      editor
        .chain()
        .focus()
        .insertContentAt(after, { type: "paragraph", content: [{ type: "text", text: "/" }] })
        .setTextSelection(after + 2)
        .run()
    }
    setHovered(null)
  }

  const select = () => {
    const { view } = editor
    view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, hovered.start)))
    view.focus()
  }

  const onDragStart = (e: React.DragEvent) => {
    const { view } = editor
    const sel = NodeSelection.create(view.state.doc, hovered.start)
    view.dispatch(view.state.tr.setSelection(sel))
    startBlockDrag(view, sel)
    e.dataTransfer.effectAllowed = "move"
    e.dataTransfer.setData("text/plain", node.type.name === CHART_NODE ? "chart" : node.textContent)
    const dom = view.nodeDOM(hovered.start) as HTMLElement | null
    if (dom) e.dataTransfer.setDragImage(dom, 0, 0)
    setDragging(true)
  }
  const onDragEnd = () => {
    endBlockDrag(editor.view)
    setDragging(false)
    setHovered(null)
  }

  const btn = cn("grid h-6 w-6 place-items-center rounded-md text-muted-foreground/70 hover:bg-muted hover:text-foreground active:scale-[0.94]", EASE, FOCUS)

  return (
    <div className="rd-handle pointer-events-none absolute z-10 hidden items-center gap-0.5 md:flex" style={{ top: hovered.top, left: hovered.left }}>
      <IconTip label="Insert below" side="bottom">
        <button type="button" aria-label="Insert below" onClick={insertBelow} className={cn(btn, "pointer-events-auto")}>
          <Plus className="size-4" aria-hidden />
        </button>
      </IconTip>
      <IconTip label="Drag to move · Alt ↑↓" side="bottom">
        <button
          type="button"
          aria-label="Move block (drag, or Alt+Up / Alt+Down)"
          draggable
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          onClick={select}
          className={cn(btn, "pointer-events-auto cursor-grab active:cursor-grabbing")}
        >
          <GripVertical className="size-4" aria-hidden />
        </button>
      </IconTip>
    </div>
  )
}

export type { SlashItem }
