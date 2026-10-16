import type {
  CalcField,
  CalcFieldService,
  CalcFieldSource,
  FormulaCheck,
  FormulaFunction,
} from "../contracts/calc-fields"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * CalcFieldService over `/api/dashboard/calc-fields`. A non-OK answer
 * becomes a `ServiceError` carrying the server's sentence (already fixed and
 * free of database text), 409 and 422 as `invalid_request`.
 */

function sourceQuery(source: CalcFieldSource): string {
  return source.mart !== undefined
    ? `mart=${encodeURIComponent(source.mart)}`
    : `source=${encodeURIComponent(source.source)}`
}

async function call<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    const kind =
      res.status === 401 || res.status === 403
        ? "permission_denied"
        : res.status === 404
          ? "not_found"
          : res.status >= 500
            ? "unavailable"
            : "invalid_request"
    throw new ServiceError(kind, json?.error ?? fallback)
  }
  return json as T
}

const jsonInit = (method: string, body: unknown, signal?: AbortSignal): RequestInit => ({
  method,
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
  signal,
})

export const calcFieldService: CalcFieldService = {
  async list(source, signal) {
    const j = await call<{ fields: CalcField[] }>(
      `/api/dashboard/calc-fields?${sourceQuery(source)}`,
      { signal },
      "The calculated fields could not be loaded."
    )
    return j.fields ?? []
  },
  validate(source, formula, name, signal) {
    return call<FormulaCheck>(
      "/api/dashboard/calc-fields/validate",
      jsonInit("POST", { ...source, formula, name }, signal),
      "The formula could not be checked."
    )
  },
  async create(source, name, formula, signal) {
    const j = await call<{ field: CalcField }>(
      "/api/dashboard/calc-fields",
      jsonInit("POST", { ...source, name, formula }, signal),
      "The field could not be saved."
    )
    return j.field
  },
  async update(id, formula, signal) {
    const j = await call<{ field: CalcField }>(
      "/api/dashboard/calc-fields",
      jsonInit("PUT", { id, formula }, signal),
      "The field could not be saved."
    )
    return j.field
  },
  async remove(id, signal) {
    await call<{ ok: boolean }>(
      `/api/dashboard/calc-fields?id=${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
      "The field could not be deleted."
    )
  },
  async functions(signal) {
    const j = await call<{ functions: FormulaFunction[] }>(
      "/api/dashboard/calc-fields/functions",
      { signal },
      "The function list could not be loaded."
    )
    return j.functions ?? []
  },
}
