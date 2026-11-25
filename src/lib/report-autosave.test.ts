import { strict as assert } from "node:assert"
import { test } from "node:test"
import { Autosave, refusalOf, type AutosaveStatus, type Timers } from "./report-autosave"

type Input = { title: string; text: string }

/** A clock the test moves by hand. */
function fakeClock() {
  let now = 0
  let next = 1
  const pending = new Map<number, { at: number; fn: () => void }>()
  const timers: Timers = {
    set: (fn, ms) => {
      const id = next++
      pending.set(id, { at: now + ms, fn })
      return id
    },
    clear: (h) => {
      pending.delete(h as number)
    },
  }
  const advance = async (ms: number) => {
    now += ms
    for (const [id, t] of [...pending].sort((a, b) => a[1].at - b[1].at)) {
      if (t.at <= now) {
        pending.delete(id)
        t.fn()
      }
    }
    await settle()
  }
  return { timers, advance, pending: () => pending.size }
}

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

/** A fake report service: each save waits until the test answers it. */
function fakeService() {
  const calls: { input: Input; id: string | null; resolve: (id: string) => void; reject: (e: unknown) => void }[] = []
  const save = (input: Input, id: string | null) =>
    new Promise<{ id: string }>((resolve, reject) => {
      calls.push({ input, id, resolve: (made) => resolve({ id: made }), reject })
    })
  return { calls, save }
}

const empty = (i: Input) => !i.title.trim() && !i.text.trim()

function make(id: string | null = null, initial?: Input) {
  const clock = fakeClock()
  const service = fakeService()
  const created: string[] = []
  const seen: AutosaveStatus[] = []
  const auto = new Autosave<Input>({
    save: service.save,
    isEmpty: empty,
    id,
    initial,
    onCreated: (made) => created.push(made),
    timers: clock.timers,
  })
  auto.subscribe(() => {
    const s = auto.get().status
    if (seen[seen.length - 1] !== s) seen.push(s)
  })
  return { auto, clock, service, created, seen }
}

test("a change is saved about a second after typing stops, not on every key", async () => {
  const { auto, clock, service, seen } = make("r1", { title: "T", text: "" })
  auto.change({ title: "T", text: "a" })
  await clock.advance(600)
  auto.change({ title: "T", text: "ab" })
  await clock.advance(600)
  assert.equal(service.calls.length, 0, "the timer restarts on each change")
  await clock.advance(400)
  assert.equal(service.calls.length, 1)
  assert.deepEqual(service.calls[0].input, { title: "T", text: "ab" })
  assert.equal(service.calls[0].id, "r1")
  service.calls[0].resolve("r1")
  await settle()
  assert.equal(auto.get().status, "saved")
  assert.deepEqual(seen, ["dirty", "saving", "saved"])
})

test("flush saves at once (blur, leaving the page) and an unchanged page is never saved", async () => {
  const { auto, clock, service } = make("r1", { title: "T", text: "" })
  auto.change({ title: "T", text: "" })
  await auto.flush()
  assert.equal(service.calls.length, 0, "the loaded text is not saved again")
  auto.change({ title: "T", text: "x" })
  const done = auto.flush()
  assert.equal(service.calls.length, 1)
  assert.equal(clock.pending(), 0, "the timer is cancelled")
  service.calls[0].resolve("r1")
  await done
  assert.equal(auto.get().status, "saved")
  assert.equal(auto.pending, false)
})

test("a new report is created on its first change that holds something, once, then updated", async () => {
  const { auto, clock, service, created } = make(null)
  auto.change({ title: "", text: "  " })
  await clock.advance(2000)
  assert.equal(service.calls.length, 0, "an empty new report is not made")
  assert.equal(auto.get().status, "idle")
  auto.change({ title: "Q3", text: "" })
  await clock.advance(1000)
  assert.equal(service.calls.length, 1)
  assert.equal(service.calls[0].id, null)
  // Typing goes on while the create is in flight, and asks to save now.
  auto.change({ title: "Q3", text: "rose" })
  void auto.flush()
  assert.equal(service.calls.length, 1, "never a second create while the first runs")
  service.calls[0].resolve("r9")
  await settle()
  assert.deepEqual(created, ["r9"])
  assert.equal(service.calls.length, 2)
  assert.equal(service.calls[1].id, "r9", "the second save updates what the first made")
  assert.deepEqual(service.calls[1].input, { title: "Q3", text: "rose" })
  service.calls[1].resolve("r9")
  await settle()
  assert.equal(auto.get().status, "saved")
  assert.equal(auto.get().id, "r9")
})

test("a change during a save is saved after it, after the delay", async () => {
  const { auto, clock, service } = make("r1")
  auto.change({ title: "T", text: "a" })
  await clock.advance(1000)
  auto.change({ title: "T", text: "ab" })
  assert.equal(auto.get().status, "saving")
  service.calls[0].resolve("r1")
  await settle()
  assert.equal(auto.get().status, "dirty")
  await clock.advance(1000)
  assert.equal(service.calls.length, 2)
  assert.deepEqual(service.calls[1].input, { title: "T", text: "ab" })
})

test("a network failure keeps the text, says Couldn't save, and Retry saves it", async () => {
  const { auto, clock, service } = make("r1")
  auto.change({ title: "T", text: "kept" })
  await clock.advance(1000)
  service.calls[0].reject(Object.assign(new Error("Failed to fetch"), { code: "unavailable" }))
  await settle()
  assert.deepEqual(auto.get(), { status: "error", refusal: null, id: "r1" })
  assert.equal(auto.pending, true, "the page warns before it is closed")
  const retried = auto.retry()
  assert.equal(service.calls.length, 2)
  assert.deepEqual(service.calls[1].input, { title: "T", text: "kept" })
  service.calls[1].resolve("r1")
  await retried
  assert.equal(auto.get().status, "saved")
})

test("a refusal shows the server's sentence beside the status, and the next change tries again", async () => {
  const { auto, clock, service } = make("r1")
  auto.change({ title: "T", text: "31 charts" })
  await clock.advance(1000)
  service.calls[0].reject(Object.assign(new Error("A report holds at most 30 charts."), { code: "invalid_request" }))
  await settle()
  assert.deepEqual(auto.get(), { status: "error", refusal: "A report holds at most 30 charts.", id: "r1" })
  auto.change({ title: "T", text: "30 charts" })
  assert.deepEqual(auto.get(), { status: "dirty", refusal: null, id: "r1" })
  await clock.advance(1000)
  assert.equal(service.calls.length, 2)
})

test("only the server's own refusals are shown as sentences", () => {
  assert.equal(refusalOf({ code: "invalid_request", message: "A link in the text must start with https://" }), "A link in the text must start with https://")
  assert.equal(refusalOf({ code: "unavailable", message: "upstream text" }), null)
  assert.equal(refusalOf(new Error("x")), null)
  assert.equal(refusalOf(null), null)
})

test("after dispose the timer is gone and nothing is told", async () => {
  const { auto, clock, service, seen } = make("r1")
  auto.change({ title: "T", text: "a" })
  const before = seen.length
  auto.dispose()
  await clock.advance(5000)
  assert.equal(service.calls.length, 0)
  assert.equal(seen.length, before)
})
