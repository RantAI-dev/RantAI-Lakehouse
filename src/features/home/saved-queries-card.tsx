"use client"

import Link from "next/link"

import { formatRelativeTime } from "@/lib/format"
import { savedQueryHref, savedQueryRows, type CardRead } from "@/lib/home-cards"
import { CardNote, CardSection, CardSkeleton, ReadNote } from "./home-ui"

type SavedQuery = { id: string; title: string; updatedAt: string }

/**
 * The most recently changed saved queries, each opening in Query Studio
 * the way `Recent` opens one. Reading them needs `query:read`, so a refusal
 * is worded as one (`ReadNote`) and not as "no saved queries".
 */
export function SavedQueriesCard({
  read,
  queries,
}: {
  read: CardRead
  queries: SavedQuery[]
}) {
  const rows = savedQueryRows(queries)
  return (
    <CardSection
      title="Saved queries"
      link={{ href: "/query-studio/saved", label: "All saved queries" }}
    >
      {read === "loading" ? (
        <CardSkeleton />
      ) : read !== "ok" ? (
        <ReadNote read={read} what="saved queries" />
      ) : rows.length === 0 ? (
        <CardNote>No saved queries yet. Save one from Query Studio.</CardNote>
      ) : (
        <ul className="flex flex-col">
          {rows.map((q) => (
            <li key={q.id}>
              <Link
                href={savedQueryHref(q.id)}
                className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
              >
                <span className="min-w-0 flex-1 truncate">{q.title}</span>
                <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                  {formatRelativeTime(q.updatedAt)}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </CardSection>
  )
}
