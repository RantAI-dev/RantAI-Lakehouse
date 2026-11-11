/**
 * `WEB-7` D3: the temporary password an invite receives, carried from the
 * Invite user page to the Users list after the redirect.
 *
 * The server returns it exactly once (`POST /api/identity/users`) and the
 * list shows it in the same one-shot dialog it always did (`ADM-3`,
 * `ADM-11`). It travels in memory only: never the URL, never
 * `localStorage`, never `sessionStorage` — a credential written to any of
 * those outlives the tab or the dialog.
 *
 * `take` clears the value as it reads it, so a re-render (or a later visit
 * to the list) does not reopen the dialog for a password that was already
 * shown.
 */
export type ShownOncePassword = {
  password: string
  mustChangePassword: boolean
}

let pending: ShownOncePassword | null = null

/** Called by the invite page once the server answered with a password. */
export function rememberInvitePassword(value: ShownOncePassword): void {
  pending = value
}

/** Read once by the Users list on mount. */
export function takeInvitePassword(): ShownOncePassword | null {
  const value = pending
  pending = null
  return value
}
