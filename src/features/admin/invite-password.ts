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
 * The value is attributed to the administrator who created it
 * (`WEB-7` S1 review round 1, S1): signing out is a `router.push`, not a
 * reload, so a plain module variable would survive the sign-out and hand
 * the password to the next person who signs in in that tab and opens
 * Users. `takeInvitePassword` therefore hands it back only to the same
 * user id, and only inside `INVITE_PASSWORD_TTL_MS`.
 *
 * `take` clears the value as it reads it — in every branch, including the
 * mismatch and expired ones — so a re-render (or a later visit to the
 * list) does not reopen the dialog for a password that was already shown
 * or is no longer allowed to be shown.
 */
export type ShownOncePassword = {
  password: string
  mustChangePassword: boolean
}

/**
 * How long a remembered password may be taken, in milliseconds.
 *
 * *(proposed)* — `WEB-7` S1 review round 1: long enough for the redirect
 * and the list's first render, short enough that the value cannot sit in
 * a tab waiting for a later visitor. The product owner may change it.
 */
export const INVITE_PASSWORD_TTL_MS = 2 * 60_000

type PendingPassword = {
  value: ShownOncePassword
  /** The `/api/auth/me` id of the signed-in administrator who invited. */
  userId: string
  /** The clock reading at `rememberInvitePassword`, from `now()`. */
  rememberedAt: number
}

let pending: PendingPassword | null = null

/**
 * Called by the invite page once the server answered with a password.
 *
 * `now` is injectable so a test can move the clock instead of sleeping.
 */
export function rememberInvitePassword(
  value: ShownOncePassword,
  userId: string,
  now = Date.now()
): void {
  pending = { value, userId, rememberedAt: now }
}

/**
 * Read once by the Users list, for the signed-in user `userId`.
 *
 * Returns the value only when it was remembered by that same user and is
 * no older than `INVITE_PASSWORD_TTL_MS`; every other case clears it and
 * returns `null`. `now` is injectable for the same reason as above.
 */
export function takeInvitePassword(userId: string, now = Date.now()): ShownOncePassword | null {
  const entry = pending
  pending = null
  if (!entry) return null
  if (entry.userId !== userId) return null
  if (now - entry.rememberedAt > INVITE_PASSWORD_TTL_MS) return null
  return entry.value
}
