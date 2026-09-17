/**
 * `GET /api/notifications` (WS5 item F1) — the navbar bell's data source.
 *
 * No `unreadCount`: there is no per-principal "read" cursor anywhere in
 * this schema, so "unread since when" has no honest answer — an earlier
 * draft invented one (WS5 plan review U11); this contract does not.
 * `openAlerts`/`pendingApprovals` are full lists, never bare counts, so
 * the bell dot can never light above an empty list underneath it.
 *
 * `supported: false` (no `openAlerts`/`pendingApprovals` at all) means no
 * Postgres pool is configured for this deployment — distinct from a
 * `supported: true` response with empty lists, which means genuinely
 * nothing is open/pending right now.
 */
export type OpenAlertNotification = {
  id: string
  title: string
  // Nullable — copied verbatim from the firing rule's own `severity`
  // column (never invented from the rule's kind); `null` when the rule
  // was saved without one.
  severity: string | null
  at: string
}

export type PendingApprovalNotification = {
  id: string
  title: string
  requestedAt: string
}

export type Notifications =
  | {
      supported: true
      openAlerts: OpenAlertNotification[]
      pendingApprovals: PendingApprovalNotification[]
    }
  | { supported: false; reason: string }

export interface NotificationsService {
  list(signal?: AbortSignal): Promise<Notifications>
}
