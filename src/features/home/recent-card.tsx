"use client"

import * as React from "react"
import Link from "next/link"
import { BarChart3, FileCode2, MessageSquare } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { formatRelativeTime } from "@/lib/format"
import type { RecentItem, RecentKind } from "@/lib/home-recent"
import { SectionTitle } from "./home-ui"

const RECENT_ICON: Record<
  RecentKind,
  { icon: React.ComponentType<{ className?: string }>; label: string }
> = {
  dashboard: { icon: BarChart3, label: "Dashboard" },
  conversation: { icon: MessageSquare, label: "Conversation" },
  query: { icon: FileCode2, label: "Saved query" },
}

/** Dashboards, conversations and saved queries as one list (`recentItems`). */
export function RecentList({
  loading,
  failed,
  items,
}: {
  loading: boolean
  failed: string[]
  items: RecentItem[]
}) {
  return (
    <section className="flex min-w-0 flex-col gap-3">
      <SectionTitle>Recent</SectionTitle>
      <div className="rounded-xl border border-border bg-card p-2">
        {loading ? (
          <div className="flex flex-col gap-2 p-2">
            <Skeleton className="h-5 w-full" />
            <Skeleton className="h-5 w-4/5" />
            <Skeleton className="h-5 w-full" />
          </div>
        ) : items.length === 0 && failed.length === 0 ? (
          <p className="px-2 py-2 text-xs text-muted-foreground">
            Nothing yet. Dashboards, conversations and saved queries you work
            on show up here.
          </p>
        ) : (
          <ul className="flex flex-col">
            {items.map((it) => {
              const { icon: Icon, label } = RECENT_ICON[it.kind]
              return (
                <li key={`${it.kind}:${it.id}`}>
                  <Link
                    href={it.href}
                    className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
                  >
                    <span title={label} className="shrink-0 text-muted-foreground">
                      <Icon className="size-4" />
                      <span className="sr-only">{label}</span>
                    </span>
                    <span className="min-w-0 flex-1 truncate">{it.title}</span>
                    <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                      {it.lastOpened
                        ? "last opened"
                        : it.at
                          ? formatRelativeTime(it.at)
                          : ""}
                    </span>
                  </Link>
                </li>
              )
            })}
          </ul>
        )}
        {!loading && failed.length > 0 ? (
          <p className="px-2 pt-1 pb-1 text-xs text-muted-foreground">
            Couldn&apos;t load {failed.join(" and ")}.
          </p>
        ) : null}
        <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 border-t border-border px-2 pt-2 pb-1 text-xs text-muted-foreground">
          <Link href="/dashboards/browse" className="underline-offset-4 hover:text-foreground hover:underline">
            Dashboards
          </Link>
          <Link href="/copilot/history" className="underline-offset-4 hover:text-foreground hover:underline">
            History
          </Link>
          <Link href="/query-studio/saved" className="underline-offset-4 hover:text-foreground hover:underline">
            Saved queries
          </Link>
        </div>
      </div>
    </section>
  )
}
