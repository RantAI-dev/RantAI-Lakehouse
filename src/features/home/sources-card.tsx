"use client"

import Link from "next/link"

import { HEALTH_LABEL, type Health } from "@/lib/status"
import { sourceRows, type CardRead } from "@/lib/home-cards"
import { cn } from "@/lib/utils"
import { CardNote, CardSection, CardSkeleton, ReadNote } from "./home-ui"

const HEALTH_DOT: Record<string, string> = {
  unhealthy: "bg-destructive",
  degraded: "bg-amber-500",
  healthy: "bg-emerald-500",
}

type Source = { id: string; name: string; type: string; health: Health }

/**
 * Sources and how each is doing, the ones that need a look first. The page
 * reads connectors once for its status line; this card shows the same
 * list. The health word is the connector's own `health`, which is only as
 * fresh as its last probe: "unknown" is shown as unknown.
 */
export function SourcesCard({
  read,
  sources,
}: {
  read: CardRead
  sources: Source[]
}) {
  const rows = sourceRows(sources)
  return (
    <CardSection title="Sources" link={{ href: "/connectors", label: "All sources" }}>
      {read === "loading" ? (
        <CardSkeleton />
      ) : read !== "ok" ? (
        <ReadNote read={read} what="sources" />
      ) : rows.length === 0 ? (
        <CardNote>No source is connected yet.</CardNote>
      ) : (
        <ul className="flex flex-col">
          {rows.map((s) => (
            <li key={s.id}>
              <Link
                href="/connectors"
                title={`${s.name} (${s.type}): ${HEALTH_LABEL[s.health] ?? s.health}`}
                className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
              >
                <span
                  className={cn(
                    "size-2 shrink-0 rounded-full",
                    HEALTH_DOT[s.health] ?? "bg-muted-foreground/40",
                  )}
                />
                <span className="min-w-0 flex-1 truncate">{s.name}</span>
                <span
                  className={cn(
                    "shrink-0 text-xs",
                    s.health === "unhealthy" ? "text-destructive" : "text-muted-foreground",
                  )}
                >
                  {HEALTH_LABEL[s.health] ?? s.health}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </CardSection>
  )
}
