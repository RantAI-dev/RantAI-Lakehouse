/**
 * Auth contracts — wire shapes shared between `services/clients/auth.ts`
 * and the features that consume it. The `auth.ts` client is the outlier
 * that still holds its login/logout types inline (those are imported
 * directly by `AuthProvider` and the login pages, not through the
 * service registry); only read-only shapes that travel to features via
 * `authService` belong here.
 *
 * Mirrors `lakehouse_store::sessions::SessionRow` in
 * `rust/crates/lakehouse-store/src/sessions.rs` — the `#[serde(rename_all
 * = "camelCase")]` on the Rust struct produces `{ id, userId, userName,
 * createdAt, expiresAt, createdIp, userAgent }` on the wire. Both times are
 * RFC 3339 in UTC with milliseconds (`ADM-14` D11), which every browser's
 * `Date` parses. `userAgent` is the browser at sign-in (`ADM-14` D9) and
 * `null` for a session older than that change; `createdIp` is always `null`
 * — the API has no trusted source for the client's address, so the page says
 * "Not recorded" rather than showing a wrong one (`ADM-14` D10).
 */
export type Session = {
  id: string;
  userId: string;
  userName: string;
  createdAt: string;
  expiresAt: string;
  createdIp: string | null;
  userAgent: string | null;
};
/**
 * `GET /api/auth/password-rules` (`ADM-11` D4): the lengths the server
 * actually enforces, so the invite form states them instead of a guess.
 */
export type PasswordRulesResponse = {
  minLength: number;
  maxLength: number;
};
