"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { authService } from "@/services"
import type { Session } from "@/services/contracts/auth"

/**
 * Sessions admin page (WS8 §Phase F). Reads `GET /api/auth/sessions`
 * (WS8 §Phase D, D1) and revokes with `DELETE /api/auth/sessions/{id}`
 * (WS8 §Phase D, D2). The page deliberately does not gate on
 * `identity:sessions:manage`: the ownership split lives server-side, so
 * any authenticated caller can render their own sessions — the revoke
 * button only succeeds for the caller's own sessions (or for any
 * session, with admin), and the route's 404-uniform behavior (Hard
 * Requirement 4) prevents the UI from being an enumeration oracle.
 */
export function SessionsPage() {
  const state = useService((signal) => authService.listSessions(signal), [])
  const revoke = useServiceAction<[string], void>(
    (signal, id) => authService.deleteSession(id, signal),
  )

  async function handleRevoke(id: string) {
    // `useServiceAction.run` resolves to `T | null`: `null` on a caught
    // error (other than an abort), the resolved value otherwise. `void`
    // for the happy path means "null only on failure".
    const ok = await revoke.run(id)
    if (ok !== null) state.reload()
  }

  const columns: ColumnDef<Session>[] = [
    { key: "user", header: "User", render: (r) => r.userName },
    {
      key: "created",
      header: "Created",
      render: (r) => formatRelativeTime(r.createdAt),
    },
    {
      key: "expires",
      header: "Expires",
      render: (r) => formatRelativeTime(r.expiresAt),
    },
    { key: "ip", header: "IP", render: (r) => r.createdIp ?? "—" },
    { key: "ua", header: "User agent", render: (r) => r.userAgent ?? "—" },
    {
      key: "actions",
      header: "",
      render: (r) => (
        <Button
          size="sm"
          variant="destructive"
          onClick={() => void handleRevoke(r.id)}
          disabled={revoke.status === "pending"}
        >
          Revoke
        </Button>
      ),
    },
  ]

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Sessions"
        description="Active browser sessions across this workspace."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={state.data}
          rowKey={(r) => r.id}
        />
      ) : null}
    </div>
  )
}