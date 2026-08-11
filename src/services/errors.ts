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

export class ServiceError extends Error {
  readonly code: ServiceErrorCode

  constructor(code: ServiceErrorCode, message: string) {
    super(message)
    this.name = "ServiceError"
    this.code = code
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
