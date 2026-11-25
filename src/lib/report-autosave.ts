/**
 * Autosave for the report editor (`BI-62`, decision D4; spec target T5): no
 * Save button. A change is saved about a second after typing stops, and at
 * once when the page asks (`flush`: the editor loses focus, the person leaves
 * the page, the tab is hidden or closed).
 *
 * A small state machine, pure and with its timer passed in, so it is tested
 * with a fake clock and a fake service (`report-autosave.test.ts`):
 *
 *     idle ──change──▶ dirty ──timer or flush──▶ saving ──▶ saved
 *                        ▲                         │
 *                        └──── change ◀── error ◀──┘ (retry = flush)
 *
 * - **One save at a time.** A change made while a save runs waits for it and
 *   is saved right after, so the first save of a new report (a create) is
 *   never sent twice and the last write is always the newest text.
 * - **Created on the first change that holds something** (a title or any
 *   block): an untouched new report is never stored. The page then takes the
 *   new id (its address becomes the edit address).
 * - **Nothing typed is lost.** A failed save keeps the text in the editor and
 *   says so: the server's own sentence when it refused (the 31st chart, an
 *   unsafe link; `invalid_request`, `permission_denied`, `not_found`), or no
 *   sentence when the network or the server failed, so the page shows
 *   "Couldn't save" with Retry. The next change, or Retry, tries again.
 * - The same text is not saved twice: a change equal to what was last saved
 *   (by its JSON) does nothing.
 *
 * Last write wins when two people edit one report, as before (a limit on the
 * feature page).
 */

export type AutosaveStatus = "idle" | "dirty" | "saving" | "saved" | "error"

export type AutosaveState = {
  status: AutosaveStatus
  /** The server's sentence for a refused save; null for a failure without one. */
  refusal: string | null
  /** The report's id once it exists. */
  id: string | null
}

/** The refusals whose message is the server's own fixed sentence (`ServiceError` codes). */
const SENTENCE_CODES = new Set(["invalid_request", "permission_denied", "not_found"])

/** The sentence a failed save shows, or null when it is not the server's refusal. */
export function refusalOf(err: unknown): string | null {
  if (!err || typeof err !== "object") return null
  const { code, message } = err as { code?: unknown; message?: unknown }
  return typeof code === "string" && SENTENCE_CODES.has(code) && typeof message === "string" && message ? message : null
}

export type Timers = {
  set: (fn: () => void, ms: number) => unknown
  clear: (handle: unknown) => void
}

const REAL_TIMERS: Timers = {
  set: (fn, ms) => globalThis.setTimeout(fn, ms),
  clear: (h) => globalThis.clearTimeout(h as ReturnType<typeof setTimeout>),
}

export type AutosaveOptions<T> = {
  /** Save `input`: create when `id` is null, else update; answers the report's id. */
  save: (input: T, id: string | null) => Promise<{ id: string }>
  /** Whether `input` holds nothing worth creating a report for. */
  isEmpty: (input: T) => boolean
  /** The id of a report that exists already (an edit); null for a new one. */
  id: string | null
  /** The input as loaded, so an unchanged page is not saved. */
  initial?: T
  /** Called once, when the first save of a new report made it. */
  onCreated?: (id: string) => void
  delayMs?: number
  timers?: Timers
}

export class Autosave<T> {
  private state: AutosaveState
  private latest: T | undefined
  private savedKey: string | undefined
  private timer: unknown = null
  private running: Promise<void> | null = null
  private again = false
  private disposed = false
  private readonly listeners = new Set<() => void>()
  private readonly delay: number
  private readonly timers: Timers

  constructor(private readonly options: AutosaveOptions<T>) {
    this.state = { status: "idle", refusal: null, id: options.id }
    this.delay = options.delayMs ?? 1000
    this.timers = options.timers ?? REAL_TIMERS
    if (options.initial !== undefined) this.savedKey = JSON.stringify(options.initial)
  }

  get = (): AutosaveState => this.state

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  /** Whether something typed is not saved yet (the page warns before it is closed). */
  get pending(): boolean {
    return this.state.status === "dirty" || this.state.status === "saving" || this.state.status === "error"
  }

  /** The page changed: remember the newest input and save it after the delay. */
  change(input: T): void {
    if (this.disposed) return
    this.latest = input
    const key = JSON.stringify(input)
    if (this.state.status !== "saving" && key === this.savedKey) {
      this.cancelTimer()
      if (this.state.status === "dirty" || this.state.status === "error") this.set({ status: this.state.id ? "saved" : "idle", refusal: null })
      return
    }
    // A new report that still holds nothing is not made.
    if (this.state.id === null && this.options.isEmpty(input)) {
      this.cancelTimer()
      if (this.state.status === "dirty") this.set({ status: "idle", refusal: null })
      return
    }
    // While a save runs, the status stays "saving"; the timer (or a flush) saves this after it.
    if (this.state.status !== "saving") this.set({ status: "dirty", refusal: null })
    this.schedule()
  }

  /** Save now what is not saved yet; resolves when it is saved or has failed. */
  flush(): Promise<void> {
    this.cancelTimer()
    if (this.running) {
      // Asked during a save: save the newest text right after it.
      this.again = true
      return this.running
    }
    if ((this.state.status !== "dirty" && this.state.status !== "error") || this.latest === undefined) return Promise.resolve()
    this.running = this.loop().finally(() => {
      this.running = null
    })
    return this.running
  }

  /** Retry after a failure: the same as saving now. */
  retry(): Promise<void> {
    return this.flush()
  }

  /** Stop the timer and stop telling the page; a save in flight still finishes. */
  dispose(): void {
    this.cancelTimer()
    this.disposed = true
    this.listeners.clear()
  }

  private async loop(): Promise<void> {
    for (;;) {
      const input = this.latest as T
      const key = JSON.stringify(input)
      this.again = false
      this.set({ status: "saving", refusal: null })
      const created = this.state.id === null
      try {
        const made = await this.options.save(input, this.state.id)
        this.savedKey = key
        this.set({ id: made.id })
      } catch (err) {
        this.set({ status: "error", refusal: refusalOf(err) })
        return
      }
      // The report exists whatever the page does with its id.
      if (created && this.state.id) {
        try {
          this.options.onCreated?.(this.state.id)
        } catch {
          // The page's follow-up (its address) is a convenience; the save stands.
        }
      }
      if (JSON.stringify(this.latest) === this.savedKey) {
        this.set({ status: "saved" })
        return
      }
      // Typed while saving: at once if a flush asked for it, else after the delay.
      if (this.again) continue
      this.set({ status: "dirty" })
      this.schedule()
      return
    }
  }

  private schedule(): void {
    this.cancelTimer()
    if (this.disposed) return
    this.timer = this.timers.set(() => {
      this.timer = null
      void this.flush()
    }, this.delay)
  }

  private cancelTimer(): void {
    if (this.timer !== null) {
      this.timers.clear(this.timer)
      this.timer = null
    }
  }

  private set(next: Partial<AutosaveState>): void {
    this.state = { ...this.state, ...next }
    if (!this.disposed) for (const l of this.listeners) l()
  }
}
