import { act, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { revealStep, useSmoothText } from "./smooth-text"

/**
 * Manual animation frames. happy-dom's `requestAnimationFrame` runs on real
 * time, which makes a pacing assertion flaky; the hook reads the timestamp of
 * each frame, so the test hands it precise ones.
 */
function controlledFrames() {
  const pending = new Map<number, FrameRequestCallback>()
  let next = 1
  const originalRaf = globalThis.requestAnimationFrame
  const originalCancel = globalThis.cancelAnimationFrame
  globalThis.requestAnimationFrame = (cb) => {
    const id = next++
    pending.set(id, cb)
    return id
  }
  globalThis.cancelAnimationFrame = (id) => {
    pending.delete(id)
  }
  return {
    /** Runs every scheduled frame at `now`, then clears them. */
    step(now: number) {
      const callbacks = [...pending.values()]
      pending.clear()
      for (const cb of callbacks) cb(now)
    },
    restore() {
      globalThis.requestAnimationFrame = originalRaf
      globalThis.cancelAnimationFrame = originalCancel
    },
  }
}

const FRAME = 1000 / 60

type Props = { target: string }

function renderReveal(initial: string, reduce = false) {
  const frames = controlledFrames()
  const hook = renderHook(({ target }: Props) => useSmoothText(target, reduce), {
    initialProps: { target: initial },
  })
  let clock = 1000
  const tick = () => {
    clock += FRAME
    act(() => frames.step(clock))
  }
  return { ...hook, frames, tick }
}

afterEach(() => {
  // `restore` runs inside each test's `finally`; nothing global is left set.
})

describe("the answer reveal keeps up with the model (AI-24 D5)", () => {
  it("trails an answer written at 300 characters a second by about 0.3 s (≤ 120 characters)", () => {
    const { result, rerender, frames, tick } = renderReveal("")
    try {
      let target = ""
      // Five seconds at 60 frames a second.
      for (let frame = 1; frame <= 300; frame += 1) {
        target = "x".repeat(Math.round((300 * frame) / 60))
        rerender({ target })
        tick()
      }
      expect(target.length).toBe(1500)
      expect(result.current.length).toBeGreaterThan(1200)
      expect(target.length - result.current.length).toBeLessThanOrEqual(120)
    } finally {
      frames.restore()
    }
  })

  it("shows a short backlog one character at a time, never nothing", () => {
    expect(revealStep(1, 0)).toBe(1)
    expect(revealStep(3, 1 / 60)).toBe(1)
    expect(revealStep(0, 1 / 60)).toBe(0)
  })
})

describe("the checked answer continues the draft (AI-24 D5)", () => {
  it("goes on from the shown text instead of appearing whole", () => {
    const draft = "a".repeat(300)
    const final = `${draft}${"b".repeat(900)}`
    const { result, rerender, frames, tick } = renderReveal(draft)
    try {
      for (let i = 0; i < 20; i += 1) tick()
      const shownBefore = result.current.length
      expect(shownBefore).toBeGreaterThan(0)

      rerender({ target: final })
      // The first paint of the checked answer is the prefix already shown.
      expect(result.current.length).toBe(shownBefore)
      expect(result.current).not.toBe(final)

      const step = revealStep(final.length - shownBefore, FRAME / 1000)
      tick()
      const grown = result.current.length - shownBefore
      expect(grown).toBeGreaterThan(0)
      expect(grown).toBeLessThanOrEqual(step)
    } finally {
      frames.restore()
    }
  })

  it("shows a different checked answer at once, as D5 says", () => {
    const draft = "a".repeat(300)
    const rewritten = "The figures could not be backed by a tool result."
    const { result, rerender, frames, tick } = renderReveal(draft)
    try {
      for (let i = 0; i < 20; i += 1) tick()
      rerender({ target: rewritten })
      expect(result.current).toBe(rewritten)
    } finally {
      frames.restore()
    }
  })

  it("shows everything at once under reduced motion", () => {
    const { result, frames } = renderReveal("the whole answer, at once", true)
    try {
      expect(result.current).toBe("the whole answer, at once")
    } finally {
      frames.restore()
    }
  })
})
