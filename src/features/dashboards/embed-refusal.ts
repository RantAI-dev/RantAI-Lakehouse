/**
 * What the signed-embed page tells a viewer when the API refuses its token
 * (SEC-12). The API answers every token refusal with one fixed message
 * (`TOKEN_REFUSED` in `rust/crates/lakehouse-api/src/routes/embed.rs`) so a
 * caller cannot tell which check failed; the page does the same. Before
 * SEC-12 a refused token read "Dashboard not available.", the same as a
 * dashboard that does not exist; this replaces that line for a signed token,
 * it does not add a second one.
 */

/** The page's line for anything that is not a token refusal. */
export const EMBED_UNAVAILABLE = "Dashboard not available."

/** `TOKEN_REFUSED`: bad signature, no or too long lifetime, expired, withdrawn. */
export const EMBED_TOKEN_REFUSED = "embed token is invalid or expired"

/** `EMBEDDING_NOT_CONFIGURED`: no `EMBED_SECRET` on the server. */
export const EMBED_NOT_CONFIGURED = "embedding is not configured"

/**
 * The line to show for a signed embed answered with `status` and `body`. The
 * wording is chosen here from the status and the body's `error`, never taken
 * from the response, so nothing but these lines can ever be shown.
 */
export function embedRefusalMessage(status: number, body: unknown): string {
  const error = typeof body === "object" && body !== null ? (body as { error?: unknown }).error : undefined
  if (status === 401 && error === EMBED_TOKEN_REFUSED) return EMBED_TOKEN_REFUSED
  if (status === 503 && error === EMBED_NOT_CONFIGURED) return EMBED_NOT_CONFIGURED
  return EMBED_UNAVAILABLE
}
