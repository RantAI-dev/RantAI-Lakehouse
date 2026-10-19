import type { CreateUploadResponse, Upload, UploadParts, UploadSession } from "@/services/contracts/uploads"

/**
 * The upload queue (`DATA-17`, plan task T1.4): sends several files to the API
 * in pieces and tells each file's progress. Pure TypeScript with no React and
 * no `fetch`: the five calls it needs arrive as a `QueueTransport`, and the
 * clock, the timer and the connection state as a `QueueEnv`, so every rule
 * below is tested with a fake of each.
 *
 * # The rules (decisions 4, 6 and 7 of `docs/core/features/upload-big-files-and-more-formats.md`)
 *
 * - **At most 4 pieces in flight, across all files.** The API admits four
 *   pieces per user (`UPLOAD_MAX_CONCURRENT_PER_USER`, `SEC-17`); the queue
 *   never asks for a fifth, and takes the next piece from the files in turn so
 *   that all of them move together. A 429 is still honoured (below), since
 *   another tab of the same user shares the four.
 * - **A failed request is tried again, by itself, with backoff.** 1 s, doubling
 *   to 30 s, for a network error, a timeout or a 5xx. The file is shown as
 *   waiting for the connection until a request succeeds. There is no limit on
 *   the tries: a person removes the file, or closes the page, to stop them.
 * - **A 429 waits as long as `Retry-After` says** and does not count as a
 *   failure: nothing is wrong, the user's four places are taken.
 * - **Any other 4xx refuses the file**, with the API's own sentence (a wrong
 *   size or an unreadable file cannot be mended by sending it again).
 * - **Offline pauses everything.** While `isOnline()` is false nothing new is
 *   started; when the connection comes back every unfinished file first reads
 *   `GET .../parts` again, so only the pieces the API does not hold are sent.
 * - **The same re-read follows every failure**, because a request that failed
 *   at the network may have been stored: the answer tells which pieces count.
 * - **Resuming ends with the page** (decision 7): a file that was being sent
 *   when the page closed is sent again from the start; its abandoned session is
 *   removed by the API after 24 hours.
 */

/** Pieces in flight at once across all files: the API's per-user limit. */
export const MAX_PIECES_IN_FLIGHT = 4

/** The first wait after a failure, doubled for each failure in a row. */
export const FIRST_RETRY_MS = 1000

/** The longest wait between two tries. */
export const MAX_RETRY_MS = 30_000

/**
 * What a file is doing. `waiting`: queued, nothing of it sent yet. `sending`:
 * a request of it is under way, or it has begun. `paused`: waiting for the
 * connection, or for the time the API asked to wait. `received`: the API has
 * the whole file and accepted it. `refused`: it will not be sent, with the
 * reason in `message`.
 */
export type QueueFileState = "waiting" | "sending" | "paused" | "received" | "refused"

export type QueueFile = {
  readonly key: string
  readonly name: string
  readonly size: number
  readonly state: QueueFileState
  /** Bytes the API has acknowledged. `size` once received. */
  readonly sentBytes: number
  /** Why a file was refused, or the trouble a paused file is waiting out. */
  readonly message: string | null
  /** The completed upload, once `received`. */
  readonly result: CreateUploadResponse | null
}

/** A file offered to the queue, with the reason it must not be sent (decided by the caller), if any. */
export type Candidate = { readonly file: File; readonly problem: string | null }

/** The calls the queue makes. `@/services` `uploadService` has all of them. */
export type QueueTransport = {
  createSession(input: { filename: string; sizeBytes: number }, signal: AbortSignal): Promise<UploadSession>
  putPart(id: string, number: number, bytes: Blob, signal: AbortSignal): Promise<unknown>
  listParts(id: string, signal: AbortSignal): Promise<UploadParts>
  complete(id: string, signal: AbortSignal): Promise<CreateUploadResponse>
  get(id: string, signal: AbortSignal): Promise<Upload>
  /** Removes an upload that is still being received, so its pieces are freed. */
  discard(id: string, signal: AbortSignal): Promise<void>
}

/** The clock, the timer and the connection: faked in tests. */
export type QueueEnv = {
  now(): number
  /** Resolves after `ms`, or at once when `signal` aborts. */
  sleep(ms: number, signal: AbortSignal): Promise<void>
  isOnline(): boolean
  /** Calls `listener` when the connection comes back; returns the way to stop. */
  onOnline(listener: () => void): () => void
}

export type QueueOptions = {
  readonly maxInFlight?: number
  readonly firstRetryMs?: number
  readonly maxRetryMs?: number
  readonly env?: Partial<QueueEnv>
}

function browserEnv(): QueueEnv {
  return {
    now: () => Date.now(),
    sleep: (ms, signal) =>
      new Promise<void>((resolve) => {
        if (signal.aborted) return resolve()
        const timer = setTimeout(done, ms)
        function done() {
          clearTimeout(timer)
          signal.removeEventListener("abort", done)
          resolve()
        }
        signal.addEventListener("abort", done)
      }),
    isOnline: () => (typeof navigator === "undefined" ? true : navigator.onLine !== false),
    onOnline: (listener) => {
      if (typeof window === "undefined") return () => {}
      window.addEventListener("online", listener)
      return () => window.removeEventListener("online", listener)
    },
  }
}

/** What a failed call told, reduced to what the queue decides on. */
type Failure = {
  /** The HTTP status, or `null` for a failure that never got an answer (the network, a timeout). */
  readonly status: number | null
  readonly message: string
  readonly retryAfterMs: number | null
}

function failureOf(err: unknown): Failure {
  const shape = (typeof err === "object" && err !== null ? err : {}) as {
    status?: unknown
    message?: unknown
    retryAfterSecs?: unknown
  }
  const status = typeof shape.status === "number" ? shape.status : null
  const message = typeof shape.message === "string" && shape.message !== "" ? shape.message : "The upload failed."
  const retryAfterMs =
    typeof shape.retryAfterSecs === "number" && Number.isFinite(shape.retryAfterSecs)
      ? Math.max(0, shape.retryAfterSecs * 1000)
      : null
  return { status, message, retryAfterMs }
}

function isAbort(err: unknown): boolean {
  return typeof err === "object" && err !== null && (err as { name?: unknown }).name === "AbortError"
}

/** A refusal that sending again cannot mend: any 4xx but a timeout (408) or a 429. */
function isFatal(failure: Failure): boolean {
  const { status } = failure
  return status !== null && status >= 400 && status < 500 && status !== 408 && status !== 429
}

type Entry = {
  key: string
  file: File
  state: QueueFileState
  message: string | null
  session: UploadSession | null
  received: Set<number>
  inFlight: Set<number>
  /** A session is being opened, or the file is being completed, or parts are being re-read. */
  busy: boolean
  /** The parts must be read again before the next piece is sent. */
  resync: boolean
  /** Failures in a row, for the backoff; reset by any success. */
  failures: number
  /** No request of this file before this time (ms on the env's clock). */
  notBefore: number
  result: CreateUploadResponse | null
  /** Ends the file: aborted when it is removed or refused. */
  controller: AbortController
  /** One controller per request under way, so a connection that came back can end the ones that hung. */
  requests: Set<AbortController>
}

export class UploadQueue {
  private readonly transport: QueueTransport
  private readonly env: QueueEnv
  private readonly maxInFlight: number
  private readonly firstRetryMs: number
  private readonly maxRetryMs: number
  private readonly entries: Entry[] = []
  private readonly listeners = new Set<() => void>()
  private readonly stopOnline: () => void
  private snapshotValue: readonly QueueFile[] = []
  private nextKey = 1
  private disposed = false
  private openingSession = false
  /** Aborted by `dispose`, so a pending wake-up timer ends with the queue. */
  private readonly lifetime = new AbortController()

  constructor(transport: QueueTransport, options: QueueOptions = {}) {
    this.transport = transport
    this.env = { ...browserEnv(), ...options.env }
    this.maxInFlight = options.maxInFlight ?? MAX_PIECES_IN_FLIGHT
    this.firstRetryMs = options.firstRetryMs ?? FIRST_RETRY_MS
    this.maxRetryMs = options.maxRetryMs ?? MAX_RETRY_MS
    this.stopOnline = this.env.onOnline(() => this.connectionBack())
  }

  // ── what the screen reads ─────────────────────────────────────────────

  /** The files, in the order they were added. The same array until something changes (for `useSyncExternalStore`). */
  getSnapshot = (): readonly QueueFile[] => this.snapshotValue

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  // ── what the screen does ──────────────────────────────────────────────

  /** Adds files. A candidate with a `problem` is listed as refused and never sent. Returns their keys. */
  add(candidates: readonly Candidate[]): string[] {
    const keys: string[] = []
    for (const { file, problem } of candidates) {
      const entry: Entry = {
        key: `file-${this.nextKey++}`,
        file,
        state: problem === null ? "waiting" : "refused",
        message: problem,
        session: null,
        received: new Set(),
        inFlight: new Set(),
        busy: false,
        resync: false,
        failures: 0,
        notBefore: 0,
        result: null,
        controller: new AbortController(),
        requests: new Set(),
      }
      this.entries.push(entry)
      keys.push(entry.key)
    }
    this.changed()
    this.pump()
    return keys
  }

  /** Takes a file out of the list. One still being received is stopped and its session removed. */
  remove(key: string): void {
    const at = this.entries.findIndex((e) => e.key === key)
    if (at < 0) return
    const [entry] = this.entries.splice(at, 1)
    entry.controller.abort()
    if (entry.session !== null && entry.state !== "received") {
      // Best effort: the API removes it anyway after 24 hours.
      void this.transport.discard(entry.session.upload.id, new AbortController().signal).catch(() => {})
    }
    this.changed()
    this.pump()
  }

  /** Takes every file out of the list (see `remove`). */
  clear(): void {
    for (const { key } of [...this.snapshotValue]) this.remove(key)
  }

  /** Stops everything and forgets the files. Sessions are left to the API's sweep: this is also what a closing page does. */
  dispose(): void {
    this.disposed = true
    this.lifetime.abort()
    this.stopOnline()
    for (const entry of this.entries) entry.controller.abort()
    this.entries.length = 0
    this.listeners.clear()
    this.snapshotValue = []
  }

  // ── the scheduler ─────────────────────────────────────────────────────

  private connectionBack(): void {
    const now = this.env.now()
    for (const entry of this.entries) {
      if (entry.state === "received" || entry.state === "refused") continue
      // Read what the API holds before sending more (the plan's rule), and do not wait out a backoff set while offline.
      entry.resync = entry.session !== null
      entry.notBefore = now
      entry.failures = 0
      // A request begun before the connection dropped may never answer. It may
      // also have been stored; the re-read above tells.
      for (const request of entry.requests) request.abort()
    }
    this.changed()
    this.pump()
  }

  private active(entry: Entry): boolean {
    return entry.state !== "received" && entry.state !== "refused"
  }

  private pieceSize(entry: Entry, number: number): number {
    const session = entry.session
    if (session === null) return 0
    const start = (number - 1) * session.partSize
    return Math.max(0, Math.min(session.partSize, entry.file.size - start))
  }

  /** Runs one request of `entry` under its own signal (see `Entry.requests`). */
  private track<T>(entry: Entry, run: (signal: AbortSignal) => Promise<T>): Promise<T> {
    const own = new AbortController()
    const stop = () => own.abort()
    entry.controller.signal.addEventListener("abort", stop)
    entry.requests.add(own)
    return run(own.signal).finally(() => {
      entry.requests.delete(own)
      entry.controller.signal.removeEventListener("abort", stop)
    })
  }

  private inFlightTotal(): number {
    return this.entries.reduce((n, e) => n + e.inFlight.size, 0)
  }

  /** Starts whatever may start now, and arms a timer for what must wait. Safe to call at any time. */
  private pump(): void {
    if (this.disposed) return
    const now = this.env.now()
    let wakeAt = Number.POSITIVE_INFINITY

    if (!this.env.isOnline()) {
      for (const entry of this.entries) {
        if (this.active(entry) && entry.state !== "paused") {
          entry.state = "paused"
          entry.message = "Waiting for the connection."
        }
      }
      this.changed()
      return
    }

    // 1. A session for the next file that has none: one at a time, they are tiny.
    for (const entry of this.entries) {
      if (!this.active(entry) || entry.session !== null || entry.busy) continue
      if (entry.notBefore > now) {
        wakeAt = Math.min(wakeAt, entry.notBefore)
      } else if (!this.openingSession) {
        void this.openSession(entry)
      }
    }

    // 2. Re-reads of what the API holds, and completions.
    for (const entry of this.entries) {
      if (!this.active(entry) || entry.session === null || entry.busy) continue
      if (entry.notBefore > now) {
        wakeAt = Math.min(wakeAt, entry.notBefore)
        continue
      }
      if (entry.inFlight.size > 0) continue
      if (entry.resync) {
        void this.resyncParts(entry)
      } else if (entry.received.size === entry.session.partCount) {
        void this.completeFile(entry)
      }
    }

    // 3. Pieces, up to the limit, always for the file with the fewest in
    // flight, so that all files move together and none waits behind another.
    let room = this.maxInFlight - this.inFlightTotal()
    const eligible = this.entries.filter(
      (e) =>
        this.active(e) && e.session !== null && !e.busy && !e.resync && e.notBefore <= now
    )
    while (room > 0) {
      let chosen: Entry | null = null
      let number: number | null = null
      for (const entry of eligible) {
        const candidate = this.nextPiece(entry)
        if (candidate === null) continue
        if (chosen === null || entry.inFlight.size < chosen.inFlight.size) {
          chosen = entry
          number = candidate
        }
      }
      if (chosen === null || number === null) break
      void this.sendPiece(chosen, number)
      room -= 1
    }

    if (Number.isFinite(wakeAt)) void this.wake(wakeAt)
    this.changed()
  }

  /** The lowest numbered piece of `entry` that is neither held by the API nor on its way, or `null`. */
  private nextPiece(entry: Entry): number | null {
    const session = entry.session
    if (session === null) return null
    for (let n = 1; n <= session.partCount; n += 1) {
      if (!entry.received.has(n) && !entry.inFlight.has(n)) return n
    }
    return null
  }

  /** Calls `pump` again at `at`. A stale timer costs one extra, harmless pump. */
  private async wake(at: number): Promise<void> {
    await this.env.sleep(Math.max(0, at - this.env.now()), this.lifetime.signal)
    this.pump()
  }

  // ── requests ──────────────────────────────────────────────────────────

  private async openSession(entry: Entry): Promise<void> {
    this.openingSession = true
    entry.busy = true
    entry.state = "sending"
    this.changed()
    try {
      const session = await this.track(entry, (signal) =>
        this.transport.createSession({ filename: entry.file.name, sizeBytes: entry.file.size }, signal)
      )
      entry.session = session
      entry.failures = 0
      entry.message = null
    } catch (err) {
      this.failed(entry, err)
    } finally {
      entry.busy = false
      this.openingSession = false
    }
    this.pump()
  }

  private async sendPiece(entry: Entry, number: number): Promise<void> {
    const session = entry.session
    if (session === null) return
    entry.inFlight.add(number)
    entry.state = "sending"
    entry.message = null
    this.changed()
    try {
      const start = (number - 1) * session.partSize
      const bytes = entry.file.slice(start, start + this.pieceSize(entry, number))
      await this.track(entry, (signal) => this.transport.putPart(session.upload.id, number, bytes, signal))
      entry.received.add(number)
      entry.failures = 0
    } catch (err) {
      entry.inFlight.delete(number)
      this.failed(entry, err)
      this.pump()
      return
    }
    entry.inFlight.delete(number)
    this.pump()
  }

  private async resyncParts(entry: Entry): Promise<void> {
    const session = entry.session
    if (session === null) return
    entry.busy = true
    this.changed()
    try {
      const parts = await this.track(entry, (signal) => this.transport.listParts(session.upload.id, signal))
      entry.received = new Set(parts.received)
      entry.resync = false
    } catch (err) {
      const failure = failureOf(err)
      if (!isAbort(err) && failure.status === 409) {
        // The upload is no longer receiving. If a "complete" whose answer was lost went through, it is stored.
        await this.recoverCompleted(entry, failure)
      } else {
        this.failed(entry, err)
      }
    } finally {
      entry.busy = false
    }
    this.pump()
  }

  private async recoverCompleted(entry: Entry, notReceiving: Failure): Promise<void> {
    const session = entry.session
    if (session === null) return
    try {
      const upload = await this.track(entry, (signal) => this.transport.get(session.upload.id, signal))
      if (upload.status === "uploaded") {
        this.finish(entry, upload)
        return
      }
      this.refuse(entry, notReceiving.message)
    } catch (err) {
      this.failed(entry, err)
    }
  }

  private async completeFile(entry: Entry): Promise<void> {
    const session = entry.session
    if (session === null) return
    entry.busy = true
    entry.state = "sending"
    this.changed()
    try {
      const done = await this.track(entry, (signal) => this.transport.complete(session.upload.id, signal))
      this.finish(entry, done)
    } catch (err) {
      const failure = failureOf(err)
      if (!isAbort(err) && failure.status === 409) {
        // Pieces missing: read what the API holds, send the rest, complete again.
        entry.resync = true
        entry.notBefore = this.env.now()
        entry.message = null
      } else {
        this.failed(entry, err)
      }
    } finally {
      entry.busy = false
    }
    this.pump()
  }

  private finish(entry: Entry, result: CreateUploadResponse): void {
    entry.result = result
    entry.state = "received"
    entry.message = null
    entry.received = new Set(Array.from({ length: entry.session?.partCount ?? 0 }, (_, i) => i + 1))
  }

  private refuse(entry: Entry, message: string): void {
    entry.state = "refused"
    entry.message = message
    // Pieces of this file still on their way are of no use, and the API's
    // session for it can go (best effort: if the API refused the file itself it
    // has removed the session already, and its answer to this is a 404).
    entry.controller.abort()
    if (entry.session !== null) {
      void this.transport.discard(entry.session.upload.id, new AbortController().signal).catch(() => {})
    }
  }

  /** Decides what a failed request means for `entry`. */
  private failed(entry: Entry, err: unknown): void {
    if (isAbort(err) || entry.controller.signal.aborted) return
    const failure = failureOf(err)
    if (isFatal(failure)) {
      this.refuse(entry, failure.message)
      return
    }
    const now = this.env.now()
    entry.state = "paused"
    if (failure.status === 429) {
      // Nothing is wrong: the user's places are taken. Wait as told.
      entry.notBefore = now + (failure.retryAfterMs ?? this.firstRetryMs)
      entry.message = "Waiting for a free place to send."
      return
    }
    // Several requests failing together are one trouble, not several: the
    // wait grows once, until the first of them has been read again.
    if (!entry.resync) entry.failures += 1
    const delay = Math.min(this.maxRetryMs, this.firstRetryMs * 2 ** (entry.failures - 1))
    entry.notBefore = now + delay
    // A request that failed at the network may have been stored: read again
    // what the API holds before sending more.
    if (entry.session !== null) entry.resync = true
    entry.message = "Waiting for the connection."
  }

  private changed(): void {
    if (this.disposed) return
    this.snapshotValue = this.entries.map((e) => ({
      key: e.key,
      name: e.file.name,
      size: e.file.size,
      state: e.state,
      sentBytes:
        e.state === "received"
          ? e.file.size
          : Array.from(e.received).reduce((sum, n) => sum + this.pieceSize(e, n), 0),
      message: e.message,
      result: e.result,
    }))
    for (const listener of this.listeners) listener()
  }
}
