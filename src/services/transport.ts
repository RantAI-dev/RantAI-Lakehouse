import { ServiceError } from "./errors"

/**
 * Shared mock transport for every mock service adapter.
 *
 * Simulates network behavior in one place — deterministic latency, abort
 * support, and an optional development-only failure rate — so pages never
 * hand-roll `setTimeout`. Swapping a mock adapter for a real HTTP/Flight
 * adapter must not change the calling page.
 */

export type MockCallOptions = {
  /** Simulated latency in ms. Defaults to 350. */
  delayMs?: number
  /** Abort signal from the caller (wired by `useService`). */
  signal?: AbortSignal
  /** 0..1 chance to reject with `unavailable` — for testing error states. */
  failRate?: number
}

/** Resolves `produce()` after a simulated delay, honoring aborts. */
export function mockCall<T>(
  produce: () => T,
  { delayMs = 350, signal, failRate = 0 }: MockCallOptions = {}
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    if (signal?.aborted) {
      reject(new ServiceError("aborted", "The request was cancelled."))
      return
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort)
      if (failRate > 0 && Math.random() < failRate) {
        reject(
          new ServiceError("unavailable", "The service is temporarily unavailable.")
        )
        return
      }
      try {
        resolve(produce())
      } catch (err) {
        reject(err)
      }
    }, delayMs)
    const onAbort = () => {
      clearTimeout(timer)
      reject(new ServiceError("aborted", "The request was cancelled."))
    }
    signal?.addEventListener("abort", onAbort, { once: true })
  })
}

/**
 * Simulated long-running operation that reports progress ticks.
 * Returns a cancel function; used for run/execute flows in mock adapters.
 */
export function mockProgress(
  onTick: (fraction: number) => void,
  onDone: () => void,
  { totalMs = 2400, tickMs = 300 }: { totalMs?: number; tickMs?: number } = {}
): () => void {
  let elapsed = 0
  const interval = setInterval(() => {
    elapsed += tickMs
    if (elapsed >= totalMs) {
      clearInterval(interval)
      onTick(1)
      onDone()
    } else {
      onTick(elapsed / totalMs)
    }
  }, tickMs)
  return () => clearInterval(interval)
}

/** Deterministic hash for stable mock metrics derived from strings. */
export function stableHash(input: string): number {
  let h = 5381
  for (let i = 0; i < input.length; i += 1) {
    h = (h * 33) ^ input.charCodeAt(i)
  }
  return Math.abs(h >>> 0)
}
