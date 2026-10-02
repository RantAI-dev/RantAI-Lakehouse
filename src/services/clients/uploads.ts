import type {
  CreateUploadResponse,
  IngestUploadInput,
  IngestUploadResponse,
  Upload,
  UploadParseOptions,
  UploadPreview,
  UploadService,
} from "../contracts/uploads"
import { apiFetch } from "../http"
import { ServiceError, type ServiceErrorCode } from "../errors"

/**
 * UploadService over `/api/uploads` (`routes/uploads.rs`), plan T10.
 *
 * Every error the API answers is `{ "error": "<sentence>" }`, a sentence
 * written for the person reading it (principle 4: nothing upstream reaches a
 * response). It becomes the `ServiceError`'s message unchanged, so a screen
 * can show it as it is; `status` keeps the HTTP code for a caller that must
 * tell a 409 from a 400.
 *
 * 401 and 403 are `permission_denied`, as `deleteConnector` and most other
 * clients read them, so `ErrorState` shows its existing "You don't have
 * access" state for a person without `connector:manage` instead of a
 * "check the input" hint that would be wrong for a refusal that has nothing
 * to do with the input.
 */

function errorCode(status: number): ServiceErrorCode {
  if (status === 401 || status === 403) return "permission_denied"
  if (status === 404) return "not_found"
  if (status >= 500) return "unavailable"
  return "invalid_request"
}

async function failure(res: Response): Promise<ServiceError> {
  const body: unknown = await res.json().catch(() => null)
  const sentence =
    typeof body === "object" && body !== null && "error" in body ? (body as { error: unknown }).error : null
  const message = typeof sentence === "string" && sentence.trim() !== "" ? sentence : `Failed (${res.status})`
  return new ServiceError(errorCode(res.status), message, res.status)
}

async function readJson<T>(res: Response): Promise<T> {
  if (!res.ok) throw await failure(res)
  return (await res.json()) as T
}

const base = "/api/uploads"

function uploadUrl(id: string, suffix = ""): string {
  return `${base}/${encodeURIComponent(id)}${suffix}`
}

export const uploadService: UploadService = {
  async list(signal) {
    return readJson<Upload[]>(await apiFetch(base, { signal }))
  },
  async get(id, signal) {
    return readJson<Upload>(await apiFetch(uploadUrl(id), { signal }))
  },
  async create(file, signal) {
    const form = new FormData()
    form.append("file", file, file.name)
    // No `Content-Type`: the browser writes `multipart/form-data` with the
    // boundary of this body, and a header set here would carry none.
    return readJson<CreateUploadResponse>(await apiFetch(base, { method: "POST", body: form, signal }))
  },
  async preview(id, options, signal) {
    // A parameter that is sent must be valid (the API refuses an empty one
    // rather than detect), so only what the caller chose is sent.
    const query = new URLSearchParams()
    const chosen: Partial<UploadParseOptions> = options ?? {}
    if (chosen.encoding !== undefined) query.set("encoding", chosen.encoding)
    if (chosen.delimiter !== undefined) query.set("delimiter", chosen.delimiter)
    if (chosen.headerRow !== undefined) query.set("headerRow", String(chosen.headerRow))
    const text = query.toString()
    return readJson<UploadPreview>(
      await apiFetch(uploadUrl(id, `/preview${text === "" ? "" : `?${text}`}`), { signal })
    )
  },
  async ingest(id, input: IngestUploadInput, signal) {
    return readJson<IngestUploadResponse>(
      await apiFetch(uploadUrl(id, "/ingest"), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(input),
        signal,
      })
    )
  },
  async remove(id, signal) {
    // 204 with no body.
    const res = await apiFetch(uploadUrl(id), { method: "DELETE", signal })
    if (!res.ok) throw await failure(res)
  },
}
