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
  /**
   * What a refusal says after "You don't have access.": the permission the
   * API named, or the server's own words. Set only by `deniedFor`;
   * `undefined` for every other error.
   */
  readonly detail?: string

  constructor(
    code: ServiceErrorCode,
    message: string,
    status?: number,
    retryAfterSecs?: number,
    detail?: string,
  ) {
    super(message)
    this.name = "ServiceError"
    this.code = code
    this.status = status
    this.retryAfterSecs = retryAfterSecs
    this.detail = detail
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

/** The fixed sentence for a refusal (WEB-11 D4). */
export const NO_ACCESS = "You don't have access."
/** The next step a toast and the permission state add after a refusal. */
export const ASK_ADMIN = "Ask a workspace admin."

// `ApiError::PermissionDenied` renders as `permission_denied: <payload>`
// (`rust/crates/lakehouse-core/src/error.rs`); the API sends no separate field.
const DENIED_CODE = /^permission_denied\s*(?::\s*([\s\S]*))?$/
const PERMISSION_ID = /^[a-z][a-z0-9_]*:[a-z][a-z0-9_]*$/

/** A refusal in words: the whole sentence, and the part after "You don't have access." */
export type Denied = { message: string; detail: string }

/** Whether `text` is the API's code-first refusal text. */
export function isDeniedText(text: string): boolean {
  return DENIED_CODE.test(text.trim())
}

/**
 * The sentence for the `error` text of a refused answer (WEB-11 D4).
 * Never throws: `raw` is whatever the body held.
 */
export function readDenied(raw: unknown): Denied {
  const text = typeof raw === "string" ? raw.trim() : ""
  const coded = DENIED_CODE.exec(text)
  if (coded === null) {
    // No code in front: words somebody wrote are shown as they are; a bare
    // word (`unauthorized`, `embedding_disabled`) or nothing is not for the reader.
    return /\s/.test(text) ? { message: text, detail: text } : { message: NO_ACCESS, detail: "" }
  }
  const rest = (coded[1] ?? "").trim()
  const ids = rest.split(",").map((id) => id.trim())
  if (rest !== "" && ids.every((id) => PERMISSION_ID.test(id))) {
    const detail =
      ids.length === 1
        ? `This needs the ${ids[0]} permission.`
        : `This needs one of these permissions: ${ids.join(", ")}.`
    return { message: `${NO_ACCESS} ${detail}`, detail }
  }
  if (/\s/.test(rest) && /[.!?]$/.test(rest)) return { message: rest, detail: rest }
  if (/\s/.test(rest)) {
    const detail = `Reason given: ${rest}.`
    return { message: `${NO_ACCESS} ${detail}`, detail }
  }
  return { message: NO_ACCESS, detail: "" }
}

/**
 * The one mapping from a refused API answer to a `ServiceError` (WEB-11):
 * `permission_denied`, the status, the sentence and its detail for a 401
 * or a 403; `null` for every other status, which the client maps itself.
 */
export function deniedFor(status: number, body: unknown): ServiceError | null {
  if (status !== 401 && status !== 403) return null
  const raw =
    typeof body === "object" && body !== null && "error" in body
      ? (body as { error: unknown }).error
      : undefined
  const { message, detail } = readDenied(raw)
  return new ServiceError("permission_denied", message, status, undefined, detail)
}

/**
 * What to say under "You don't have access" for `error`: the detail
 * `deniedFor` set, or the one read from a message that is still the API's
 * code-first text (a client not yet on `deniedFor`); "" when there is none.
 */
export function deniedDetail(error: ServiceError): string {
  if (error.detail !== undefined) return error.detail
  return isDeniedText(error.message) ? readDenied(error.message).detail : ""
}
