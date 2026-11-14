"use client"

/**
 * The small pieces of the Dashboards page (`BI-59`, layout C of the
 * 2026-11-14 mockups): the page frame that ends above the assistant bar, the
 * large search, and (later tasks) the badges, the owner avatar, the segmented
 * type filter and the designed empty states. Kept apart from the page so the
 * table and the gallery draw them the same way.
 */
import * as React from "react"
import { Code2, FileText, Globe, LayoutDashboard, LayoutTemplate, Lock, Package, Search, Star, X, type LucideIcon } from "lucide-react"

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useAuth } from "@/features/auth/auth-provider"
import { cn } from "@/lib/utils"
import type { LibraryItem, LibraryKind } from "./library-items"
import { PRIVATE_TIP } from "./private-badge"

/** One easing and duration for every hover, press and change (owner taste: 150–250 ms, eased, never linear). */
export const EASE = "transition-[background-color,border-color,color,box-shadow,opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)]"
/** The keyboard focus ring every custom control here uses. */
export const FOCUS = "outline-none focus-visible:ring-3 focus-visible:ring-ring/50"

/**
 * Whether the assistant bar is on this page. It is the same test the app
 * frame makes (`app-frame.tsx`): the bar shows for a holder of
 * `assistant:use` or `assistant:ask`, and `/dashboards/browse` is not one of
 * the paths that hide it.
 */
export function useHasDock(): boolean {
  const { hasPermission } = useAuth()
  return hasPermission("assistant:use") || hasPermission("assistant:ask")
}

/**
 * The page as a frame that ends above the assistant bar, so the bar never
 * covers a row or a card (spec `BI-59` T5): the list scrolls inside the frame
 * instead of under the bar. Its height is the viewport less the navbar (4rem),
 * the shell's top padding (1, 1.25 or 1.5rem) and the room kept under the
 * content.
 *
 * BI-59 review round 1, SHOULD-FIX 2: that room is read from the app shell's
 * `--content-bottom`, the token BI-58 adds (`c4d87211`, `50295df6`: the
 * composer's measured height plus its offset and gap while it shows, 1.5rem
 * when it hides). Until that lands the variable is absent and the fallbacks
 * are today's shell: with the bar, a 5rem dock spacer after `main` and a
 * `pb-24` / `lg:pb-28` bottom padding that the negative margin gives back;
 * without it, the shell's own bottom padding. With the token the frame's height
 * alone puts its end where the shell says; the margin there works out at
 * `--content-bottom - 6rem` (or `- 7rem`) against a `main` that pads by
 * `--content-bottom`, which only shortens `main`'s box below the frame and
 * moves nothing on screen. It goes when this branch has the token.
 *
 * Below `lg` the whole frame scrolls, header included, so a phone keeps the
 * room for the rows; from `lg` only the list does (`ScrollRegion`).
 */
export function PageFrame({ children, className }: { readonly children: React.ReactNode; readonly className?: string }) {
  const dock = useHasDock()
  return (
    <div
      className={cn(
        "flex min-h-0 flex-col overflow-y-auto lg:overflow-hidden",
        dock
          ? cn(
              "h-[calc(100dvh-5rem-var(--content-bottom,5rem))] sm:h-[calc(100dvh-5.25rem-var(--content-bottom,5rem))] lg:h-[calc(100dvh-5.5rem-var(--content-bottom,5rem))]",
              "mb-[calc(var(--content-bottom,0px)-6rem)] lg:mb-[calc(var(--content-bottom,0px)-7rem)]",
            )
          : "h-[calc(100dvh-5rem-var(--content-bottom,1rem))] sm:h-[calc(100dvh-5.25rem-var(--content-bottom,1.25rem))] lg:h-[calc(100dvh-5.5rem-var(--content-bottom,1.5rem))]",
        className,
      )}
    >
      {children}
    </div>
  )
}

/** The part of the frame that scrolls on a laptop; its bottom edge fades instead of cutting a row in half. */
export function ScrollRegion({ children, className }: { readonly children: React.ReactNode; readonly className?: string }) {
  return (
    <div
      className={cn(
        "lg:min-h-0 lg:flex-1 lg:overflow-y-auto lg:[mask-image:linear-gradient(to_bottom,black_calc(100%-20px),transparent)] lg:[scrollbar-gutter:stable]",
        className,
      )}
    >
      {children}
    </div>
  )
}

/**
 * The page's search: large, with a clear button. "/" focuses it from
 * anywhere on the page (unless the person is typing somewhere else), and
 * Escape clears it.
 */
export function SearchField({
  value, onChange, placeholder = "Search everything", className,
}: {
  readonly value: string
  readonly onChange: (value: string) => void
  readonly placeholder?: string
  readonly className?: string
}) {
  const ref = React.useRef<HTMLInputElement>(null)
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey) return
      const target = e.target
      if (target instanceof Element && target.closest("input,textarea,select,[contenteditable=true],[role=dialog],[role=menu]")) return
      e.preventDefault()
      ref.current?.focus()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
  return (
    <div className={cn("relative", className)}>
      <Search aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4.5 -translate-y-1/2 text-muted-foreground" />
      <input
        ref={ref}
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && value) {
            e.preventDefault()
            onChange("")
          }
        }}
        placeholder={placeholder}
        aria-label="Search dashboards, reports and templates"
        className={cn(
          "h-11 w-full min-w-0 rounded-lg bg-muted/70 pr-16 pl-9 text-[15px] placeholder:text-muted-foreground hover:bg-muted focus:bg-background focus:ring-1 focus:ring-ring/60 dark:bg-input/30 dark:focus:bg-input/40 [&::-webkit-search-cancel-button]:hidden",
          EASE,
          "outline-none focus-visible:ring-3 focus-visible:ring-ring/40",
        )}
      />
      {value ? (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => {
            onChange("")
            ref.current?.focus()
          }}
          className={cn("absolute top-1/2 right-2 grid size-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground", EASE, FOCUS)}
        >
          <X className="size-3.5" aria-hidden />
        </button>
      ) : (
        <kbd aria-hidden className="pointer-events-none absolute top-1/2 right-3 hidden h-5 -translate-y-1/2 items-center rounded bg-background px-1.5 text-[11px] text-muted-foreground shadow-xs sm:inline-flex dark:bg-input/40">
          /
        </kbd>
      )}
    </div>
  )
}

/* ------------------------------------------------------------------ */
/* Kinds, badges and people                                            */
/* ------------------------------------------------------------------ */

const KIND_ICON: Record<LibraryKind, LucideIcon> = { dashboard: LayoutDashboard, report: FileText, template: LayoutTemplate }

export function KindIcon({ kind, className }: { readonly kind: LibraryKind; readonly className?: string }) {
  const Icon = KIND_ICON[kind]
  return <Icon aria-hidden className={cn("size-4 shrink-0 text-muted-foreground", className)} />
}

/**
 * The badge tones (decision P4): one colour per meaning, the same on every
 * row and card. Built-in grey, public link amber, embedded sky; rose is kept
 * for "restricted" so the meaning has its colour, but the list does not show
 * it: whether a folder shows only the people listed is read per item
 * (`GET /api/dashboard/access`), and the boards and folders lists do not carry
 * it.
 */
type Tone = "neutral" | "amber" | "sky" | "rose"
const TONE: Record<Tone, string> = {
  neutral: "bg-muted text-muted-foreground",
  amber: "bg-amber-500/12 text-amber-700 dark:text-amber-300",
  sky: "bg-sky-500/12 text-sky-700 dark:text-sky-300",
  rose: "bg-rose-500/12 text-rose-700 dark:text-rose-300",
}

/** A label that cannot be clicked: soft fill, no border (DESIGN.md "Badge tanpa border", owner 2026-11-08). */
export function Badge({ tone, icon: Icon, children, className }: {
  readonly tone: Tone
  readonly icon?: LucideIcon
  readonly children: React.ReactNode
  readonly className?: string
}) {
  return (
    <span className={cn("inline-flex h-5 items-center gap-1 rounded-full px-2 text-[11px] font-medium whitespace-nowrap", TONE[tone], className)}>
      {Icon ? <Icon aria-hidden className="size-3" /> : null}
      {children}
    </span>
  )
}

/**
 * Who can see an item beyond the folder it is in: built in, a live public
 * link, embedding on, or private to its owner's personal space (the lock, as
 * before, with the same tip as the dashboard's own badge).
 */
export function ItemBadges({ item, className, max }: {
  readonly item: LibraryItem
  readonly className?: string
  /** At most this many, on a card where room is short. */
  readonly max?: number
}) {
  const list: React.ReactNode[] = []
  if (item.builtin) list.push(<Badge key="builtin" tone="neutral" icon={Package}>Built-in</Badge>)
  if (item.publicLink) list.push(<Badge key="public" tone="amber" icon={Globe}>Public link</Badge>)
  if (item.embedded) list.push(<Badge key="embed" tone="sky" icon={Code2}>Embedded</Badge>)
  if (item.personal) {
    list.push(
      <Tooltip key="private">
        <TooltipTrigger render={<span className="inline-flex" />}>
          <Badge tone="neutral" icon={Lock}>Private</Badge>
        </TooltipTrigger>
        <TooltipContent>{PRIVATE_TIP}</TooltipContent>
      </Tooltip>,
    )
  }
  if (list.length === 0) return null
  return <span className={cn("flex flex-wrap items-center gap-1", className)}>{max ? list.slice(0, max) : list}</span>
}

const AVATAR_TONES = [
  "bg-sky-500/15 text-sky-800 dark:text-sky-200",
  "bg-emerald-500/15 text-emerald-800 dark:text-emerald-200",
  "bg-amber-500/15 text-amber-800 dark:text-amber-200",
  "bg-violet-500/15 text-violet-800 dark:text-violet-200",
]

/** Initials in a circle, a colour per name; the built-in board has a box instead of a person. */
export function Avatar({ owner, builtin, className }: { readonly owner: string; readonly builtin?: boolean; readonly className?: string }) {
  if (builtin) {
    return (
      <span aria-hidden className={cn("grid size-5 shrink-0 place-items-center rounded-full bg-muted text-muted-foreground", className)}>
        <Package className="size-3" />
      </span>
    )
  }
  const initials = owner.split(/\s+/).filter(Boolean).map((w) => w[0]).slice(0, 2).join("").toUpperCase() || "?"
  const tone = AVATAR_TONES[[...owner].reduce((sum, c) => sum + c.charCodeAt(0), 0) % AVATAR_TONES.length]
  return (
    <span aria-hidden className={cn("grid size-5 shrink-0 place-items-center rounded-full text-[9px] font-semibold", tone, className)}>
      {initials}
    </span>
  )
}

/** The owner's name as the list says it: "You" for the person reading, "Built-in" for the built-in board, "—" when unknown. */
export function ownerLabel(item: Pick<LibraryItem, "owner" | "builtin">, myName: string | undefined): string {
  if (item.builtin) return "Built-in"
  if (!item.owner) return "—"
  return myName && item.owner === myName ? "You" : item.owner
}

/** A row of mutually exclusive choices; each is a real button with `aria-pressed`, reachable by Tab. */
export function Segmented<T extends string>({
  value, options, onChange, label, size = "default", className,
}: {
  readonly value: T
  readonly options: readonly { value: T; label: string }[]
  readonly onChange: (value: T) => void
  /** Read by screen readers. */
  readonly label: string
  readonly size?: "sm" | "default"
  readonly className?: string
}) {
  return (
    <div role="group" aria-label={label} className={cn("inline-flex rounded-lg bg-muted p-0.5", className)}>
      {options.map((o) => {
        const on = o.value === value
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={on}
            onClick={() => onChange(o.value)}
            className={cn(
              "inline-flex flex-1 items-center justify-center gap-1.5 rounded-md font-medium whitespace-nowrap",
              size === "sm" ? "h-7 px-2.5 text-xs" : "h-8 px-3 text-sm",
              EASE, FOCUS,
              on ? "bg-background text-foreground shadow-sm dark:bg-input/40" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {o.label}
          </button>
        )
      })}
    </div>
  )
}

/** The favourite star: filled amber when on, with a short pop when it changes. */
export function StarButton({ on, name, onToggle, className }: {
  readonly on: boolean
  readonly name: string
  readonly onToggle: () => void
  readonly className?: string
}) {
  const [pop, setPop] = React.useState(false)
  React.useEffect(() => {
    if (!pop) return
    const t = window.setTimeout(() => setPop(false), 220)
    return () => window.clearTimeout(t)
  }, [pop])
  return (
    <button
      type="button"
      aria-pressed={on}
      aria-label={on ? `Remove ${name} from favourites` : `Add ${name} to favourites`}
      onClick={(e) => {
        e.preventDefault()
        e.stopPropagation()
        onToggle()
        setPop(true)
      }}
      className={cn(
        "relative z-10 grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground active:scale-90",
        EASE, FOCUS, className,
      )}
    >
      <Star aria-hidden className={cn("size-4", EASE, on && "fill-amber-400 text-amber-500", pop && "scale-125")} />
    </button>
  )
}
