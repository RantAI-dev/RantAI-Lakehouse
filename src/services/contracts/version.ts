/**
 * The server's own health answer (`GET /health`) — the console's only
 * window on the build the server runs (REL-2 decision D7).
 *
 * The API answers this route whenever the process can answer at all: it
 * is a liveness probe, not a readiness one. The console reads it for one
 * thing — the `version` — so it can say when the console and the server
 * are not the same build.
 */
export type ServerHealth = {
  /** `ok` for any process that answers this route. */
  status: string
  /** `ok` once the application database is usable, `unavailable` otherwise. */
  database: string
  /** The version the server runs (`CARGO_PKG_VERSION` of the API). */
  version: string
}

/**
 * The console's view of the server it talks to.
 */
export interface VersionService {
  /** The server's health answer, including the version it runs. */
  getServerHealth(signal?: AbortSignal): Promise<ServerHealth>
}
