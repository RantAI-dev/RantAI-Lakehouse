"use client"

import * as React from "react"
import { useService } from "@/hooks/use-service"
import { readableErrorMessage } from "@/lib/notify-message"
import { queryService } from "@/services"
import type { SavedQueryAccessList } from "@/services/contracts/queries"

/**
 * The names of the people and roles a folder is shared with, other than the
 * caller.
 *
 * A grantee can sit in the folder's own rules and in the rules it inherits
 * from a folder above, so rules are folded by grantee and each one counts
 * once. The caller is left out by user id: the line warns about other
 * people, and the author of a folder is always in its list. A role is never
 * the caller. The server's name is shown as it came; a grantee it found no
 * name for is "Unknown person" or "Unknown role", never a made-up name.
 */
function sharedWith(
  list: SavedQueryAccessList,
  viewerId: string | undefined,
): string[] {
  const found = new Map<string, string>()
  for (const rule of [...list.rules, ...list.inherited]) {
    if (rule.roleId === null && viewerId !== undefined && rule.userId === viewerId) continue
    const key = rule.roleId ? `role:${rule.roleId}` : `user:${rule.userId ?? ""}`
    if (found.has(key)) continue
    const unknown = rule.roleId ? "Unknown role" : "Unknown person"
    found.set(key, rule.granteeName ?? unknown)
  }
  return [...found.values()]
}

/**
 * Who a folder is shared with, in one line, under the picker that chose it.
 *
 * A query saved or moved into a folder takes the folder's rules with it, and
 * the person choosing the folder cannot change those rules, so the dialog
 * tells them before they confirm (`QS-3`). `folderId` is `null` where there
 * is nothing to say and nothing to ask: the shared space (everyone in the
 * organisation sees it at view) and the caller's own space (it holds no
 * rule).
 *
 * The note never blocks the save or the move: while the list loads it shows
 * nothing, and a failed load shows the server's sentence on its own line.
 */
export function FolderSharingNote({
  folderId,
  viewerId,
}: {
  /** The folder the item will land in; `null` for a space that holds no rule. */
  readonly folderId: string | null
  /** The signed-in person, who is left out of the line; `undefined` leaves nobody out. */
  readonly viewerId: string | undefined
}) {
  const access = useService(
    (signal) =>
      folderId === null
        ? Promise.resolve(null as SavedQueryAccessList | null)
        : queryService.getFolderAccess(folderId, signal),
    [folderId],
  )

  if (access.status === "error") {
    return (
      <p className="text-xs text-destructive">
        {readableErrorMessage(access.error.message)}
      </p>
    )
  }
  if (access.status !== "success" || access.data === null) return null

  const grantees = sharedWith(access.data, viewerId)
  if (grantees.length === 0) return null
  const count = grantees.length === 1 ? "1 person or role" : `${grantees.length} people or roles`
  return (
    <p className="text-xs text-muted-foreground">
      Shared with {count}: {grantees.join(", ")}.
    </p>
  )
}
