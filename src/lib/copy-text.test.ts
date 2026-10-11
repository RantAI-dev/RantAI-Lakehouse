import { afterEach, describe, expect, it, mock } from "bun:test"
import { copyText } from "./copy-text"

const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard")
const originalExec = document.execCommand

afterEach(() => {
  if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard)
  else Reflect.deleteProperty(navigator, "clipboard")
  document.execCommand = originalExec
  Object.defineProperty(globalThis, "isSecureContext", { value: true, configurable: true })
})

function setClipboard(value: unknown) {
  Object.defineProperty(navigator, "clipboard", { value, configurable: true })
}

describe("copyText", () => {
  it("uses the async clipboard when it is there", async () => {
    const writeText = mock(async () => {})
    setClipboard({ writeText })
    expect(await copyText("hello")).toBe(true)
    expect(writeText).toHaveBeenCalledWith("hello")
  })

  it("falls back to a temporary textarea when there is no clipboard (an http origin)", async () => {
    setClipboard(undefined)
    let copied = ""
    document.execCommand = mock((cmd: string) => {
      copied = (document.activeElement as HTMLTextAreaElement).value
      return cmd === "copy"
    }) as unknown as typeof document.execCommand
    expect(await copyText("from http")).toBe(true)
    expect(copied).toBe("from http")
    expect(document.querySelector("textarea")).toBeNull()
  })

  it("falls back when the context is not secure even if the object exists", async () => {
    const writeText = mock(async () => {})
    setClipboard({ writeText })
    Object.defineProperty(globalThis, "isSecureContext", { value: false, configurable: true })
    document.execCommand = mock(() => true) as unknown as typeof document.execCommand
    expect(await copyText("x")).toBe(true)
    expect(writeText).not.toHaveBeenCalled()
  })

  it("falls back when the async write is refused", async () => {
    setClipboard({ writeText: async () => { throw new Error("denied") } })
    document.execCommand = mock(() => true) as unknown as typeof document.execCommand
    expect(await copyText("x")).toBe(true)
  })

  it("resolves false when nothing worked, so the caller can say so", async () => {
    setClipboard(undefined)
    document.execCommand = mock(() => false) as unknown as typeof document.execCommand
    expect(await copyText("x")).toBe(false)
    document.execCommand = mock(() => { throw new Error("no") }) as unknown as typeof document.execCommand
    expect(await copyText("x")).toBe(false)
    expect(document.querySelector("textarea")).toBeNull()
  })
})
