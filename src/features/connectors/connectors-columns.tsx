"use client"

import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { HealthBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import { HEALTH_LABEL, type Health } from "@/lib/status"
import type { Connector } from "@/services/contracts/connectors"

type Direction = Connector["direction"]

export const DIRECTION_LABEL: Record<Direction, string> = {
  source: "Source",
  sink: "Sink",
  bidirectional: "Bidirectional",
}

const DIRECTION_OPTIONS = (Object.keys(DIRECTION_LABEL) as Direction[]).map((d) => ({
  value: d,
  label: DIRECTION_LABEL[d],
}))

const HEALTH_OPTIONS = (Object.keys(HEALTH_LABEL) as Health[]).map((h) => ({
  value: h,
  label: HEALTH_LABEL[h],
}))

/**
 * The later of a connector's last successful and last failed run, with which
 * one it was; `null` when it has neither (no run has reported yet, which the
 * list says as "No runs yet" rather than showing a blank). The two come from
 * the run reports (`SRC-7`), not from a manual Test.
 */
export function latestRun(
  c: Pick<Connector, "lastRunSuccessAt" | "lastRunFailureAt">
): { at: string; succeeded: boolean } | null {
  const { lastRunSuccessAt: ok, lastRunFailureAt: failed } = c
  if (!ok && !failed) return null
  if (!ok) return { at: failed as string, succeeded: false }
  if (!failed) return { at: ok, succeeded: true }
  // A timestamp that does not parse loses to one that does; a tie is a failure,
  // the safer thing to show.
  return Date.parse(ok) > Date.parse(failed)
    ? { at: ok, succeeded: true }
    : { at: failed, succeeded: false }
}

/** "3 failed in a row" for a streak above zero, `null` otherwise. */
export function failureStreakLabel(streak: number): string | null {
  return streak > 0 ? `${streak} failed in a row` : null
}

/**
 * The Sources list's columns. A connector opens at `/connectors/<id>`: the
 * name and the row menu's "View details" are links, and a press on the rest of
 * the row opens it too (`onRowClick` in `connectors-page.tsx`), so the row's
 * own controls stop their clicks from reaching the row.
 */
export function getConnectorColumns(): ColumnDef<Connector>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Connector" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <Link
              href={`/connectors/${r.id}`}
              // Without this a press on the name also reaches the row's own
              // press, which would push the page a second time, and a
              // "new tab" click would open it in this tab as well.
              onClick={(event) => event.stopPropagation()}
              className="text-left font-medium hover:underline focus-visible:underline focus:outline-none"
            >
              {r.name}
            </Link>
            <p className="text-xs text-muted-foreground">{r.type}</p>
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Connector",
        variant: "text",
      },
    },
    {
      accessorKey: "direction",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Direction" />
      ),
      cell: ({ row }) => <span>{DIRECTION_LABEL[row.original.direction]}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Direction",
        variant: "select",
        options: DIRECTION_OPTIONS,
      },
    },
    {
      accessorKey: "health",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Health" />
      ),
      cell: ({ row }) => {
        const streak = failureStreakLabel(row.original.failureStreak)
        return (
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
            <HealthBadge health={row.original.health} />
            {streak ? <span className="text-xs text-destructive">{streak}</span> : null}
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Health",
        variant: "select",
        options: HEALTH_OPTIONS,
      },
    },
    {
      accessorKey: "environment",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Environment" />
      ),
      cell: ({ row }) => <span>{row.original.environment}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Environment",
        variant: "text",
      },
    },
    {
      accessorKey: "tenant",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tenant" />
      ),
      cell: ({ row }) => <span>{row.original.tenant}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Tenant",
        variant: "text",
      },
    },
    {
      accessorKey: "lastTestAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last test" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastTestAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Last test",
        variant: "date",
      },
    },
    {
      id: "lastRun",
      accessorFn: (c) => latestRun(c)?.at ?? null,
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last run" />
      ),
      cell: ({ row }) => {
        const last = latestRun(row.original)
        if (!last) return <span className="text-muted-foreground">No runs yet</span>
        return (
          <span className="text-muted-foreground">
            <span className={last.succeeded ? "text-foreground" : "text-destructive"}>
              {last.succeeded ? "Succeeded" : "Failed"}
            </span>{" "}
            {formatRelativeTime(last.at)}
          </span>
        )
      },
      meta: {
        label: "Last run",
      },
    },
    {
      accessorKey: "lastActivityAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last activity" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastActivityAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Last activity",
        variant: "date",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const item = row.original

        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8 p-0"
                  aria-label="Connector actions"
                  // The row opens the connector on a press, so without this
                  // the click that opens this menu also leaves the page.
                  onClick={(event) => event.stopPropagation()}
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                className="w-44"
                // Same reason as the trigger: a click in the (portaled) menu
                // still bubbles to the row through the React tree.
                onClick={(event) => event.stopPropagation()}
              >
                <DropdownMenuLabel>Connector</DropdownMenuLabel>
                <DropdownMenuItem asChild>
                  <Link href={`/connectors/${item.id}`}>View details</Link>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem asChild>
                  <Link href={`/pipelines/create?connectorId=${item.id}`}>
                    Create pipeline
                  </Link>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
      enableSorting: false,
      enableHiding: false,
      size: 40,
    },
  ]
}
