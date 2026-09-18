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
 * createdAt, expiresAt, createdIp, userAgent }` on the wire. `createdIp`
 * and `userAgent` round-trip as honest `null` for every session minted
 * today (both `routes/auth.rs::login` and `routes/auth.rs::oidc_callback`
 * pass `None` for them), so the shape keeps the null rather than a
 * fabricated placeholder (WS8 §Phase D).
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