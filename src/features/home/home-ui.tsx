"use client"

import * as React from "react"
import Link from "next/link"
import { ArrowRight, type LucideIcon } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { EASE, FOCUS } from "@/features/dashboards/dashboard-header"
import type { CardRead } from "@/lib/home-cards"
import { cn } from "@/lib/utils"

export { EASE, FOCUS }

/**
 * The surface every Home widget sits on (`WEB-17`): the dashboard page's
 * soft tile (`SOFT_TILE_FRAME`, `BI-61`), so Home and a dashboard read as
 * one product. No border, a fill a step off the page, a large radius
 * (DESIGN.md §3, cards without borders). The card itself is not clickable,
 * so it has no hover of its own; its rows and links do.
 */
export const WIDGET = "flex min-w-0 flex-col rounded-2xl bg-muted/45 p-4 dark:bg-card"

/** A widget's title row: an icon and the name, and at most one quiet link at the right. */
export function WidgetHead({
  title,
  icon: Icon,
  action,
  className,
}: {
  title: React.ReactNode
  icon?: LucideIcon
  action?: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex h-6 shrink-0 items-center justify-between gap-2", className)}>
      <h2 className="flex min-w-0 items-center gap-1.5 text-[13px] leading-5 font-medium tracking-normal text-foreground">
        {Icon ? <Icon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" /> : null}
        <span className="truncate">{title}</span>
      </h2>
      {action}
    </div>
  )
}

/** The quiet link at the right of a widget's title ("Open", "All chats"). */
export function WidgetLink({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <Link
      href={href}
      className={cn(
        "-mr-1.5 inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground active:scale-[0.97] dark:hover:bg-muted/60",
        EASE,
        FOCUS,
      )}
    >
      {children}
      <ArrowRight aria-hidden className="size-3" />
    </Link>
  )
}

/** One clickable line in a widget list: soft hover, the press scale, the focus ring. */
export const ROW = cn(
  "flex min-w-0 items-center gap-2.5 rounded-lg px-1.5 text-left hover:bg-muted active:scale-[0.99] dark:hover:bg-muted/50",
  EASE,
  FOCUS,
)

/**
 * The optional list cards (Sources, Open alerts, Saved queries) in the
 * widget look: the title row with its link, then the list straight on the
 * surface (no panel inside the panel).
 */
export function CardSection({
  title,
  icon,
  link,
  children,
}: {
  title: string
  icon?: LucideIcon
  link?: { href: string; label: string }
  children: React.ReactNode
}) {
  return (
    <section className={WIDGET}>
      <WidgetHead
        title={title}
        icon={icon}
        action={link ? <WidgetLink href={link.href}>{link.label}</WidgetLink> : undefined}
      />
      <div className="mt-2 -mx-1.5">{children}</div>
    </section>
  )
}

/** The text for an empty or failed widget: one short line. */
export function CardNote({ children }: { children: React.ReactNode }) {
  return <p className="px-1.5 py-1.5 text-xs leading-5 text-muted-foreground">{children}</p>
}

/** A list widget's loading look: rows in the shape of the rows to come. */
export function CardSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div aria-hidden className="flex flex-col">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex h-9 items-center gap-2.5 px-1.5">
          <Skeleton className="size-2 rounded-full" />
          <Skeleton className={cn("h-3", i % 2 ? "w-1/2" : "w-2/3")} />
          <Skeleton className="ml-auto h-3 w-10" />
        </div>
      ))}
    </div>
  )
}

/**
 * What a card says when its read did not succeed. A refusal is not an
 * empty list: "no alerts" would read as good news about something this
 * person was never allowed to look at, so the two are worded apart.
 */
export function ReadNote({
  read,
  what,
}: {
  read: Exclude<CardRead, "loading" | "ok">
  what: string
}) {
  return (
    <CardNote>
      {read === "denied" ? `You don't have access to ${what}.` : `Couldn't load ${what}.`}
    </CardNote>
  )
}
