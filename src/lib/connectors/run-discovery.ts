import type { DiscoveryAnswer, DiscoveryReadKind } from "@/services/contracts/connectors"

/** How often a listing that runs on the orchestrator is asked for its answer. */
export const DISCOVERY_POLL_MS = 1_500

/**
 * The longest the console waits for a running request. The API itself turns a request
 * that has run for five minutes into a `timeout` failure, so this only ends a wait the
 * API never ended (a lost connection that kept the poll alive).
 */
const MAX_WAIT_MS = 330_000

const SUPERSEDED: DiscoveryAnswer["reason"] = "Another request for this connector replaced this one. Try again."
const GAVE_UP: DiscoveryAnswer["reason"] = "The listing did not finish in time."

function sleep(ms: number, signal: AbortSignal | undefined): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(new DOMException("aborted", "AbortError"))
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort)
      resolve()
    }, ms)
    const onAbort = () => {
      clearTimeout(timer)
      reject(new DOMException("aborted", "AbortError"))
    }
    signal?.addEventListener("abort", onAbort, { once: true })
  })
}

/**
 * Ask for a listing or a preview and wait for its end (`SRC-13` decisions D1 and D9).
 *
 * SQL sources, object storage and the REST endpoint list answer in the first call. The
 * sources only the orchestrator reads (MongoDB, Kafka, SFTP, Oracle, and a REST/Kafka/
 * MongoDB/SFTP preview) answer `running` there, and the latest answer of that kind is read
 * every `pollMs` until it ends. The API keeps ONE answer per connector and kind, so a
 * preview whose `object` is not the one asked for was replaced by another request; it is
 * reported as a failure of its own words, never shown as the answer to this one.
 *
 * `onWaiting` is told the milliseconds waited since the first answer, for a progress line.
 *
 * # Errors
 *
 * Rejects with whatever `start` or `get` rejects with, and with an `AbortError` when
 * `signal` aborts.
 */
export async function runDiscovery({
  kind,
  start,
  get,
  signal,
  pollMs = DISCOVERY_POLL_MS,
  maxWaitMs = MAX_WAIT_MS,
  onWaiting,
  now = Date.now,
}: {
  kind: DiscoveryReadKind
  start: () => Promise<DiscoveryAnswer>
  get: () => Promise<DiscoveryAnswer>
  signal?: AbortSignal
  pollMs?: number
  maxWaitMs?: number
  onWaiting?: (waitedMs: number) => void
  now?: () => number
}): Promise<DiscoveryAnswer> {
  const first = await start()
  if (first.state !== "running") return first
  const began = now()
  for (;;) {
    onWaiting?.(now() - began)
    await sleep(pollMs, signal)
    const answer = await get()
    if (answer.state !== "running") {
      if (kind === "preview" && answer.object !== first.object) {
        return { kind, state: "failed", errorClass: "failed", reason: SUPERSEDED }
      }
      return answer
    }
    if (now() - began > maxWaitMs) {
      return { kind, state: "failed", errorClass: "timeout", reason: GAVE_UP }
    }
  }
}
