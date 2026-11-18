import type {
  NewUsageRate,
  UsageExport,
  UsageQuery,
  UsageRate,
  UsageRates,
  UsageReport,
  UsageService,
} from "../contracts/usage"
import { apiFetch } from "../http"
import { ServiceError, deniedFor } from "../errors"
import { fileNameOf } from "@/lib/browser-file"

/**
 * The usage sums, their CSV file and the rates over `/api/usage/*`
 * (`OPS-8` part B). An error body's text is the route's own fixed sentence
 * (a bad date, a period over 366 days, a second currency), never database
 * text, so it is safe to show as it is.
 */
const BASE = "/api/usage"

function errorFor(status: number, json: { error?: string } | null, failure: string): ServiceError {
  // A refusal (`tenant=all` without `*:*`, a rate write without
  // `settings:write`) goes through deniedFor, so the page says it as one.
  const denied = deniedFor(status, json)
  if (denied) return denied
  const message = json?.error ?? failure
  if (status === 400 || status === 422) return new ServiceError("invalid_request", message, status)
  return new ServiceError("unavailable", message, status)
}

/** The query string both reads share: the period, the group and, for every tenant, `tenant=all`. */
function queryString(query: UsageQuery): string {
  const params = new URLSearchParams({ from: query.from, to: query.to, group: query.group })
  if (query.tenant === "all") params.set("tenant", "all")
  return params.toString()
}

async function send<T>(
  method: string,
  url: string,
  body: unknown,
  failure: string,
  signal?: AbortSignal
): Promise<T> {
  const res = await apiFetch(url, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  })
  const json = await res.json().catch(() => null)
  if (!res.ok) throw errorFor(res.status, json, failure)
  return json as T
}

export const usageService: UsageService = {
  getUsage(query, signal) {
    return send<UsageReport>("GET", `${BASE}/queries?${queryString(query)}`, undefined, "Failed to load usage", signal)
  },
  async exportUsageCsv(query, signal): Promise<UsageExport> {
    const res = await apiFetch(`${BASE}/queries/export?${queryString(query)}`, {
      signal,
      cache: "no-store",
    })
    if (!res.ok) {
      const json = await res.json().catch(() => null)
      throw errorFor(res.status, json, "The export failed.")
    }
    return {
      blob: await res.blob(),
      fileName: fileNameOf(res.headers.get("content-disposition"), `usage-${query.from}-${query.to}.csv`),
    }
  },
  listRates(signal) {
    return send<UsageRates>("GET", `${BASE}/rates`, undefined, "Failed to load the rates", signal)
  },
  addRate(rate: NewUsageRate, signal) {
    return send<UsageRate>("POST", `${BASE}/rates`, rate, "Failed to add the rate", signal)
  },
}
