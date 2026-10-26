import { describe, expect, it } from "bun:test"
import type { CreateUploadResponse, Upload, UploadSession } from "@/services/contracts/uploads"
import {
  FIRST_RETRY_MS,
  MAX_PIECES_IN_FLIGHT,
  MAX_RETRY_MS,
  type QueueFile,
  type QueueTransport,
  UploadQueue,
} from "./upload-queue"

// The queue is tested with a fake transport and a fake clock: no network, no
// real timers. The fake session cuts files into pieces of 10 bytes, so a file
// of 50 bytes is five pieces.

const PART = 10

function upload(over: Partial<Upload> = {}): Upload {
  return {
    id: "up-1",
    originalFilename: "a.csv",
    sizeBytes: 0,
    uploadedBy: "Ana",
    status: "receiving",
    createdAt: "2026-10-19T10:00:00Z",
    updatedAt: "2026-10-19T10:00:00Z",
    ...over,
  }
}

function fileOf(name: string, bytes: number): File {
  return new File([new Uint8Array(bytes)], name)
}

/** A status error, shaped as a `ServiceError` is. */
function httpError(status: number, message = `answered ${status}`, retryAfterSecs?: number) {
  return Object.assign(new Error(message), { status, retryAfterSecs })
}

type Deferred = { resolve: (v?: unknown) => void; reject: (e: unknown) => void }

class Harness {
  now = 0
  online = true
  private sleeps: { at: number; resolve: () => void }[] = []
  private onlineListener: (() => void) | null = null
  readonly sleptFor: number[] = []

  calls: string[] = []
  inFlight = 0
  maxInFlight = 0
  /** Optional: how each call is answered; the default succeeds. */
  putPart: (id: string, n: number, signal: AbortSignal) => Promise<unknown> = async () => ({})
  listParts: (id: string) => Promise<{ received: number[]; partSize: number; partCount: number }> = async (id) => ({
    received: [...(this.stored.get(id) ?? [])].sort((a, b) => a - b),
    partSize: PART,
    partCount: this.counts.get(id) ?? 0,
  })
  complete: (id: string) => Promise<CreateUploadResponse> = async (id) => ({ ...upload({ id, status: "uploaded" }) })
  createSession: (name: string, size: number) => Promise<UploadSession> | null = () => null
  getUpload: (id: string) => Promise<Upload> = async (id) => upload({ id, status: "uploaded" })
  readonly discarded: string[] = []
  readonly signals = new Map<string, AbortSignal[]>()
  private ids = 0
  private counts = new Map<string, number>()
  /** What the fake API holds, by upload id. */
  stored = new Map<string, Set<number>>()

  readonly transport: QueueTransport = {
    createSession: async ({ filename, sizeBytes }, signal) => {
      this.calls.push(`session ${filename}`)
      const custom = this.createSession(filename, sizeBytes)
      if (custom) return custom
      this.ids += 1
      const id = `up-${this.ids}`
      const count = Math.max(1, Math.ceil(sizeBytes / PART))
      this.counts.set(id, count)
      this.stored.set(id, new Set())
      this.signals.set(id, [signal])
      return { upload: upload({ id, originalFilename: filename }), partSize: PART, partCount: count }
    },
    putPart: async (id, n, bytes, signal) => {
      this.calls.push(`put ${id}/${n}`)
      this.inFlight += 1
      this.maxInFlight = Math.max(this.maxInFlight, this.inFlight)
      try {
        expect(bytes.size).toBeGreaterThan(0)
        const answer = await this.putPart(id, n, signal)
        this.stored.get(id)?.add(n)
        return answer
      } finally {
        this.inFlight -= 1
      }
    },
    listParts: async (id) => {
      this.calls.push(`parts ${id}`)
      return this.listParts(id)
    },
    complete: async (id) => {
      this.calls.push(`complete ${id}`)
      return this.complete(id)
    },
    get: async (id) => {
      this.calls.push(`get ${id}`)
      return this.getUpload(id)
    },
    discard: async (id) => {
      this.discarded.push(id)
    },
  }

  queue = new UploadQueue(this.transport, {
    env: {
      now: () => this.now,
      isOnline: () => this.online,
      onOnline: (listener) => {
        this.onlineListener = listener
        return () => {
          this.onlineListener = null
        }
      },
      sleep: (ms, signal) =>
        new Promise<void>((resolve) => {
          this.sleptFor.push(ms)
          const entry = { at: this.now + ms, resolve }
          this.sleeps.push(entry)
          signal.addEventListener("abort", () => resolve())
        }),
    },
  })

  /** Lets every pending promise run. */
  async settle() {
    for (let i = 0; i < 30; i += 1) await new Promise((r) => setTimeout(r, 0))
  }

  /** Moves the clock and wakes the timers that came due. */
  async advance(ms: number) {
    this.now += ms
    const due = this.sleeps.filter((s) => s.at <= this.now)
    this.sleeps = this.sleeps.filter((s) => s.at > this.now)
    for (const s of due) s.resolve()
    await this.settle()
  }

  goOnline() {
    this.online = true
    this.onlineListener?.()
  }

  files(): readonly QueueFile[] {
    return this.queue.getSnapshot()
  }
}

describe("UploadQueue", () => {
  it("cuts a file into the pieces the session names, sends each, completes it and reports it received", async () => {
    const h = new Harness()
    h.queue.add([{ file: fileOf("a.csv", 25), problem: null }])
    await h.settle()

    expect(h.calls).toEqual(["session a.csv", "put up-1/1", "put up-1/2", "put up-1/3", "complete up-1"])
    const [file] = h.files()
    expect(file.state).toBe("received")
    expect(file.sentBytes).toBe(25)
    expect(file.result?.id).toBe("up-1")
  })

  it("never has more than four pieces in flight across all files, and takes them from the files in turn", async () => {
    const h = new Harness()
    const gates: Deferred[] = []
    h.putPart = () =>
      new Promise((resolve, reject) => {
        gates.push({ resolve, reject })
      })
    h.queue.add([
      { file: fileOf("a.csv", 50), problem: null },
      { file: fileOf("b.csv", 50), problem: null },
    ])
    await h.settle()

    expect(MAX_PIECES_IN_FLIGHT).toBe(4)
    expect(h.inFlight).toBe(4)
    // The first file's session arrives first and takes the places...
    const put = (id: string) => h.calls.filter((c) => c.startsWith(`put ${id}/`)).length
    expect(put("up-1")).toBe(4)
    expect(put("up-2")).toBe(0)

    // ...but the places freed go to the file with the fewest pieces in flight,
    // so the second file is not left waiting behind the first.
    gates[0].resolve()
    gates[1].resolve()
    await h.settle()
    expect(h.inFlight).toBe(4)
    expect(put("up-2")).toBe(2)
    expect(h.maxInFlight).toBe(4)

    // Release everything that is or will be in flight.
    for (let i = 0; i < 40 && h.files().some((f) => f.state !== "received"); i += 1) {
      gates.splice(0).forEach((g) => g.resolve())
      await h.settle()
    }
    expect(h.maxInFlight).toBe(4)
    expect(h.files().map((f) => f.state)).toEqual(["received", "received"])
  })

  it("reports a file's progress as the bytes the API acknowledged over its size", async () => {
    const h = new Harness()
    const gates: Deferred[] = []
    h.putPart = () =>
      new Promise((resolve, reject) => {
        gates.push({ resolve, reject })
      })
    h.queue.add([{ file: fileOf("a.csv", 25), problem: null }])
    await h.settle()
    expect(h.files()[0].state).toBe("sending")
    expect(h.files()[0].sentBytes).toBe(0)

    gates[0].resolve() // piece 1 of 10 bytes
    await h.settle()
    expect(h.files()[0].sentBytes).toBe(10)
    gates[1].resolve()
    await h.settle()
    expect(h.files()[0].sentBytes).toBe(20)
    gates[2].resolve() // the last piece holds 5 bytes
    await h.settle()
    expect(h.files()[0].state).toBe("received")
    expect(h.files()[0].sentBytes).toBe(25)
  })

  it("tries a failed piece again by itself: 1 s, doubling, never more than 30 s, and says it is waiting for the connection", async () => {
    const h = new Harness()
    let failures = 0
    h.putPart = async () => {
      if (failures < 8) {
        failures += 1
        throw new TypeError("Failed to fetch")
      }
      return {}
    }
    h.queue.add([{ file: fileOf("a.csv", 5), problem: null }])
    await h.settle()
    expect(h.files()[0].state).toBe("paused")
    expect(h.files()[0].message).toBe("Waiting for the connection.")

    for (let i = 0; i < 8; i += 1) await h.advance(MAX_RETRY_MS)
    expect(h.files()[0].state).toBe("received")
    // The waits before the pieces were tried again (the re-read in between has none of its own).
    const waits = h.sleptFor.filter((ms) => ms > 0)
    expect(waits.slice(0, 7)).toEqual([1000, 2000, 4000, 8000, 16000, 30000, 30000])
    expect(FIRST_RETRY_MS).toBe(1000)
  })

  it("re-reads the pieces the API holds after a failure and does not send again one it already has", async () => {
    const h = new Harness()
    let first = true
    h.putPart = async (id, n) => {
      if (n === 2 && first) {
        first = false
        // The piece was stored, then the answer was lost.
        h.stored.get(id)?.add(2)
        throw new TypeError("Failed to fetch")
      }
      return {}
    }
    h.queue.add([{ file: fileOf("a.csv", 25), problem: null }])
    await h.settle()
    await h.advance(FIRST_RETRY_MS)

    expect(h.calls).toContain("parts up-1")
    expect(h.calls.filter((c) => c === "put up-1/2").length).toBe(1)
    expect(h.files()[0].state).toBe("received")
  })

  it("waits as long as Retry-After says on a 429, without counting it as a failure and without re-reading", async () => {
    const h = new Harness()
    let refused = true
    h.putPart = async () => {
      if (refused) {
        refused = false
        throw httpError(429, "Too many uploads are in progress. Try again in a moment.", 5)
      }
      return {}
    }
    h.queue.add([{ file: fileOf("a.csv", 5), problem: null }])
    await h.settle()
    expect(h.files()[0].state).toBe("paused")
    expect(h.files()[0].message).toBe("Waiting for a free place to send.")

    await h.advance(4000)
    expect(h.files()[0].state).toBe("paused")
    await h.advance(1000)
    expect(h.files()[0].state).toBe("received")
    expect(h.sleptFor).toContain(5000)
    expect(h.calls.filter((c) => c.startsWith("parts"))).toEqual([])
  })

  it("starts nothing while offline, and when the connection returns reads what the API holds and sends only the rest", async () => {
    const h = new Harness()
    h.online = false
    h.queue.add([{ file: fileOf("a.csv", 40), problem: null }])
    await h.settle()
    expect(h.calls).toEqual([])
    expect(h.files()[0].state).toBe("paused")
    expect(h.files()[0].message).toBe("Waiting for the connection.")

    h.goOnline()
    await h.settle()
    expect(h.files()[0].state).toBe("received")

    // Offline in the middle: pieces 1 and 2 are stored, one request hangs, and
    // the API also took piece 3 (its answer never came).
    const g = new Harness()
    let calls = 0
    g.putPart = (id, n, signal) => {
      calls += 1
      if (calls <= 2) {
        g.stored.get(id)?.add(n)
        return Promise.resolve({})
      }
      return new Promise((_resolve, reject) => {
        signal.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")))
      })
    }
    g.queue.add([{ file: fileOf("b.csv", 40), problem: null }])
    await g.settle()
    g.online = false
    g.stored.get("up-1")?.add(3)
    g.putPart = async () => ({}) // the network answers again
    g.goOnline() // the connection is back; the hung requests are ended
    await g.settle()
    expect(g.calls).toContain("parts up-1")
    expect(g.calls.filter((c) => c === "put up-1/3").length).toBe(1)
    expect(g.files()[0].state).toBe("received")
  })

  it("refuses a file the API refuses with its own sentence, removes its session, and carries on with the others", async () => {
    const h = new Harness()
    h.putPart = async (id, n) => {
      if (id === "up-1" && n === 1) {
        throw httpError(400, "Every piece must be exactly 10 bytes, except the last, which holds the rest of the file.")
      }
      return {}
    }
    h.queue.add([
      { file: fileOf("a.csv", 25), problem: null },
      { file: fileOf("b.csv", 25), problem: null },
    ])
    await h.settle()

    const [a, b] = h.files()
    expect(a.state).toBe("refused")
    expect(a.message).toBe("Every piece must be exactly 10 bytes, except the last, which holds the rest of the file.")
    expect(h.discarded).toContain("up-1")
    expect(b.state).toBe("received")
  })

  it("lists a file the caller marked as a problem as refused and sends nothing of it", async () => {
    const h = new Harness()
    h.queue.add([{ file: fileOf("big.csv", 5), problem: "big.csv is over the limit." }])
    await h.settle()
    expect(h.calls).toEqual([])
    expect(h.files()[0]).toMatchObject({ state: "refused", message: "big.csv is over the limit.", sentBytes: 0 })
  })

  it("on a 409 at completion reads the pieces, sends the missing one and completes again", async () => {
    const h = new Harness()
    let completions = 0
    h.complete = async (id) => {
      completions += 1
      if (completions === 1) {
        h.stored.get(id)?.delete(2) // the API lost it
        throw httpError(409, "Some pieces have not arrived: 2.")
      }
      return upload({ id, status: "uploaded" })
    }
    h.queue.add([{ file: fileOf("a.csv", 25), problem: null }])
    await h.settle()

    expect(h.calls.filter((c) => c === "put up-1/2").length).toBe(2)
    expect(h.calls.filter((c) => c === "complete up-1").length).toBe(2)
    expect(h.files()[0].state).toBe("received")
  })

  it("recovers a file whose completion went through but whose answer was lost", async () => {
    const h = new Harness()
    let first = true
    h.complete = async () => {
      if (first) {
        first = false
        throw new TypeError("Failed to fetch")
      }
      throw httpError(409, "This upload is not receiving pieces.")
    }
    h.listParts = async () => {
      throw httpError(409, "This upload is not receiving pieces.")
    }
    h.queue.add([{ file: fileOf("a.csv", 5), problem: null }])
    await h.settle()
    await h.advance(FIRST_RETRY_MS)

    expect(h.calls).toContain("get up-1")
    expect(h.files()[0].state).toBe("received")
    expect(h.files()[0].result?.status).toBe("uploaded")
  })

  it("stops a file that is removed: its requests are cancelled and its session is removed; a received one is just dropped", async () => {
    const h = new Harness()
    let seen: AbortSignal | null = null
    h.putPart = (_id, _n, signal) => {
      seen = signal
      return new Promise(() => {})
    }
    const [key] = h.queue.add([{ file: fileOf("a.csv", 25), problem: null }])
    await h.settle()
    h.queue.remove(key)
    expect((seen as AbortSignal | null)?.aborted).toBe(true)
    expect(h.discarded).toEqual(["up-1"])
    expect(h.files()).toEqual([])

    const done = new Harness()
    const [doneKey] = done.queue.add([{ file: fileOf("b.csv", 5), problem: null }])
    await done.settle()
    done.queue.remove(doneKey)
    expect(done.discarded).toEqual([])
  })

  it("keeps the same snapshot until something changes, and tells subscribers when it does", async () => {
    const h = new Harness()
    const before = h.queue.getSnapshot()
    expect(h.queue.getSnapshot()).toBe(before)
    let told = 0
    const stop = h.queue.subscribe(() => {
      told += 1
    })
    h.queue.add([{ file: fileOf("a.csv", 5), problem: null }])
    await h.settle()
    expect(told).toBeGreaterThan(0)
    expect(h.queue.getSnapshot()).not.toBe(before)
    const after = h.queue.getSnapshot()
    expect(h.queue.getSnapshot()).toBe(after)
    stop()
  })

  it("does nothing more after it is disposed", async () => {
    const h = new Harness()
    h.putPart = () => new Promise(() => {})
    h.queue.add([{ file: fileOf("a.csv", 25), problem: null }])
    await h.settle()
    const calls = h.calls.length
    h.queue.dispose()
    h.goOnline()
    await h.advance(60_000)
    expect(h.calls.length).toBe(calls)
    expect(h.queue.getSnapshot()).toEqual([])
  })
})
