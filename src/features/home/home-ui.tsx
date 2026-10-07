"use client"

import * as React from "react"
import Link from "next/link"

import { Skeleton } from "@/components/ui/skeleton"
import type { CardRead } from "@/lib/home-cards"

/** The small uppercase heading over each card on Home's second screen. */
export function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="px-1 text-xs font-semibold tracking-[0.08em] text-muted-foreground uppercase">
      {children}
    </h2>
  )
}

/**
 * The chrome the list cards share: a title (with an optional link at its
 * end) over a bordered panel. `PipelineRuns` and `RecentList` keep their
 * own markup, which this matches, so a new card sits among them without a
 * visible seam.
 */
export function CardSection({
  title,
  link,
  children,
}: {
  title: string
  link?: { href: string; label: string }
  children: React.ReactNode
}) {
  return (
    <section className="flex min-w-0 flex-col gap-3">
      {link ? (
        <div className="flex items-baseline justify-between gap-3 px-1">
          <SectionTitle>{title}</SectionTitle>
          <Link
            href={link.href}
            className="shrink-0 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            {link.label}
          </Link>
        </div>
      ) : (
        <SectionTitle>{title}</SectionTitle>
      )}
      <div className="rounded-xl border border-border bg-card p-2">{children}</div>
    </section>
  )
}

/** The panel text for an empty or failed card. */
export function CardNote({ children }: { children: React.ReactNode }) {
  return <p className="px-2 py-2 text-xs text-muted-foreground">{children}</p>
}

export function CardSkeleton() {
  return (
    <div className="flex flex-col gap-2 p-2">
      <Skeleton className="h-5 w-full" />
      <Skeleton className="h-5 w-4/5" />
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
      {read === "denied"
        ? `You don't have access to ${what}.`
        : `Couldn't load ${what}.`}
    </CardNote>
  )
}
