import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  FULLSCREEN_DARK_KEY, enterFullscreen, leaveFullscreen, readFullscreenDark, writeFullscreenDark, type FullscreenDoc,
} from "./dashboard-fullscreen"

function fakeDoc(over: Partial<FullscreenDoc> & { request?: () => Promise<void> } = {}): FullscreenDoc & { calls: string[] } {
  const calls: string[] = []
  const { request, ...rest } = over
  return {
    calls,
    fullscreenEnabled: true,
    fullscreenElement: null,
    documentElement: { requestFullscreen: request ?? (async () => { calls.push("request") }) },
    exitFullscreen: async () => { calls.push("exit") },
    ...rest,
  }
}

test("the browser's full screen is used when the API allows it", async () => {
  const doc = fakeDoc()
  assert.equal(await enterFullscreen(doc), "native")
  assert.deepEqual(doc.calls, ["request"])
})

test("the in-page view is the answer when the API is missing, disabled or refused", async () => {
  assert.equal(await enterFullscreen(fakeDoc({ documentElement: {} })), "in-page")
  assert.equal(await enterFullscreen(fakeDoc({ fullscreenEnabled: false })), "in-page")
  assert.equal(await enterFullscreen(fakeDoc({ fullscreenEnabled: undefined })), "in-page")
  const refused = fakeDoc({ request: async () => { throw new TypeError("Permissions check failed") } })
  assert.equal(await enterFullscreen(refused), "in-page")
})

test("a disabled API is never even asked", async () => {
  const doc = fakeDoc({ fullscreenEnabled: false })
  await enterFullscreen(doc)
  assert.deepEqual(doc.calls, [])
})

test("leaving exits only a document that is in full screen, and never throws", async () => {
  const out = fakeDoc()
  await leaveFullscreen(out)
  assert.deepEqual(out.calls, [])
  const inside = fakeDoc({ fullscreenElement: {} })
  await leaveFullscreen(inside)
  assert.deepEqual(inside.calls, ["exit"])
  const failing = fakeDoc({ fullscreenElement: {}, exitFullscreen: async () => { throw new Error("not in fullscreen") } })
  await leaveFullscreen(failing)
})

test("the dark choice is remembered as true/false and read back", () => {
  const store = new Map<string, string>()
  const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) }
  assert.equal(readFullscreenDark(storage), false)
  writeFullscreenDark(true, storage)
  assert.equal(store.get(FULLSCREEN_DARK_KEY), "true")
  assert.equal(readFullscreenDark(storage), true)
  writeFullscreenDark(false, storage)
  assert.equal(readFullscreenDark(storage), false)
})

test("a storage that throws costs only the memory", () => {
  const broken = {
    getItem: () => { throw new Error("blocked") },
    setItem: () => { throw new Error("blocked") },
  }
  assert.equal(readFullscreenDark(broken), false)
  assert.doesNotThrow(() => writeFullscreenDark(true, broken))
})
