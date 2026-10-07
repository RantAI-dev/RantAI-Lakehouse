"use client"

import Link from "next/link"

import { formatRelativeTime } from "@/lib/format"
import { openAlertRows, type CardRead } from "@/lib/home-cards"
import { SEVERITY_LABEL, type Severity } from "@/lib/status"
import { cn } from "@/lib/utils"
import { CardNote, CardSection, CardSkeleton, ReadNote } from "./home-ui"

type Alert = {
  id: string
  title: string
  severity: Severity | null
  status: string
  at: string
}

/**
 * Open alerts, the most severe first. It reads the same list the status
 * line counts from, so the two cannot disagree. A severity the rule was
 * saved without is shown as no severity, not guessed.
 */
export function OpenAlertsCard({
  read,
  alerts,
}: {
  read: CardRead
  alerts: Alert[]
}) {
  const rows = openAlertRows(alerts)
  return (
    <CardSection title="Open alerts" link={{ href: "/alerts", label: "All alerts" }}>
      {read === "loading" ? (
        <CardSkeleton />
      ) : read !== "ok" ? (
        <ReadNote read={read} what="alerts" />
      ) : rows.length === 0 ? (
        <CardNote>No alert is open.</CardNote>
      ) : (
        <ul className="flex flex-col">
          {rows.map((a) => (
            <li key={a.id}>
              <Link
                href="/alerts"
                title={a.title}
                className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors hover:bg-muted/60"
              >
                <span
                  className={cn(
                    "size-2 shrink-0 rounded-full",
                    a.severity === "critical" || a.severity === "high"
                      ? "bg-destructive"
                      : a.severity === "medium"
                        ? "bg-amber-500"
                        : "bg-muted-foreground/40",
                  )}
                />
                <span className="min-w-0 flex-1 truncate">{a.title}</span>
                <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                  {a.severity ? SEVERITY_LABEL[a.severity] : "No severity"} ·{" "}
                  {formatRelativeTime(a.at)}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </CardSection>
  )
}
