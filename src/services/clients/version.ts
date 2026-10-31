import type { ServerHealth, VersionService } from "../contracts/version"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * The server's health answer, over the console's own origin.
 *
 * The API's health route lives at `/health` — NOT under `/api/` — so the
 * console cannot reach it through the `/api/:path*` rewrite. `next.config.ts`
 * exposes it as `/api/health` with a rewrite of its own, placed before the
 * general one; the console's own `/health` PAGE (the platform dashboard) is
 * a filesystem route and is served before any rewrite, so the two cannot
 * collide.
 *
 * A failure here is not worth an error state in the UI: the console
 * simply does not know the server's version, and the version banner stays
 * hidden (the caller decides what to do with a missing answer).
 */
async function getServerHealth(signal?: AbortSignal): Promise<ServerHealth> {
  const res = await apiFetch("/api/health", { signal })
  const json = await res.json()
  if (!res.ok) {
    throw new ServiceError("unavailable", json?.error ?? "Failed to read the server's health")
  }
  return json as ServerHealth
}

export const httpVersionService: VersionService = {
  getServerHealth,
}
