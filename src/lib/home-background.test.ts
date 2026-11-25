import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  DEFAULT_HOME_BACKGROUND,
  drawsScene,
  HOME_BACKGROUND_KEY,
  parseHomeBackground,
  readHomeBackground,
  writeHomeBackground,
} from "./home-background"

function memory(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial))
  return {
    data,
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  }
}

const throwing = {
  getItem: (): string | null => {
    throw new Error("SecurityError")
  },
  setItem: (): void => {
    throw new Error("QuotaExceededError")
  },
}

test("Dither is the default when nothing is saved", () => {
  assert.equal(DEFAULT_HOME_BACKGROUND, "dither")
  assert.equal(readHomeBackground(memory()), "dither")
  assert.equal(readHomeBackground(undefined), "dither")
})

test("each known choice reads back as itself", () => {
  for (const v of ["dither", "ascii", "off"] as const) {
    assert.equal(parseHomeBackground(v), v)
    assert.equal(readHomeBackground(memory({ [HOME_BACKGROUND_KEY]: v })), v)
  }
})

test("an unknown or empty stored value falls back to the default instead of hiding the scene", () => {
  assert.equal(parseHomeBackground(""), "dither")
  assert.equal(parseHomeBackground("OFF"), "dither")
  assert.equal(parseHomeBackground("plasma"), "dither")
  assert.equal(parseHomeBackground(null), "dither")
})

test("a write is read back by the next read", () => {
  const s = memory()
  assert.equal(writeHomeBackground(s, "off"), true)
  assert.equal(s.data.get(HOME_BACKGROUND_KEY), "off")
  assert.equal(readHomeBackground(s), "off")
  writeHomeBackground(s, "ascii")
  assert.equal(readHomeBackground(s), "ascii")
})

test("storage that throws reads as the default and a refused write reports false", () => {
  assert.equal(readHomeBackground(throwing), "dither")
  assert.equal(writeHomeBackground(throwing, "off"), false)
  assert.equal(writeHomeBackground(undefined, "off"), false)
})

test("only Off draws no scene", () => {
  assert.equal(drawsScene("dither"), true)
  assert.equal(drawsScene("ascii"), true)
  assert.equal(drawsScene("off"), false)
})
