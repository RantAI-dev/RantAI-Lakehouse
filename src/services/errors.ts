/**
 * Normalized service error surface. Every adapter (mock now, HTTP later)
 * must reject with a `ServiceError` so pages can render consistent states.
 */

export type ServiceErrorCode =
  | "not_found"
  | "permission_denied"
  | "unavailable"
  | "invalid_request"
  | "aborted"
  | "too_many_requests"

export class ServiceError extends Error {
  readonly code: ServiceErrorCode
  /**
   * The raw HTTP status a client adapter received, when there was one (a
   * mock adapter or a thrown non-HTTP failure leaves this `undefined`).
   * `code` alone collapses 400/409/422 into one `"invalid_request"` bucket
   * — too coarse for a caller that must react differently to a 409
   * (someone else changed the resource first; reload and retry) than a
   * 422 (the request was well-formed but the server could not carry it
   * out, e.g. `connector-edit-page.tsx`'s probe-first credential save).
   * Callers needing that distinction read `status` directly instead of
   * parsing `message` text.
   */
  readonly status?: number
  /** Seconds until the caller should retry, from a `Retry-After` header on
   *  a 429 response; `undefined` for every other error. */
  readonly retryAfterSecs?: number

  constructor(
    code: ServiceErrorCode,
    message: string,
    status?: number,
    retryAfterSecs?: number,
  ) {
    super(message)
    this.name = "ServiceError"
    this.code = code
    this.status = status
    this.retryAfterSecs = retryAfterSecs
  }
}

/** Narrowing helper for catch blocks. */
export function isServiceError(err: unknown): err is ServiceError {
  return err instanceof ServiceError
}

/** Wraps any thrown value into a ServiceError for uniform handling. */
export function toServiceError(err: unknown): ServiceError {
  if (isServiceError(err)) return err
  if (err instanceof DOMException && err.name === "AbortError") {
    return new ServiceError("aborted", "The request was cancelled.")
  }
  const message = err instanceof Error ? err.message : "Unexpected error."
  return new ServiceError("unavailable", message)
}
