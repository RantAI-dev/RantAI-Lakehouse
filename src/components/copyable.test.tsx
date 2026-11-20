import { cleanup, fireEvent, render, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"

import { Copyable } from "./copyable"

/**
 * `UX-6`: `Copyable` wrote through `navigator.clipboard` directly, which does
 * not exist over plain http, so the success toast fired only when the API was
 * there and the catch turn a missing API into a raw error description. It
 * goes through `copyText` now, and says why when the fallback is refused.
 *
 * `navigator.clipboard` is removed for the whole file (the staging situation)
 * and `document.execCommand` stands in for the browser's answer.
 */

const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard")

beforeEach(() => {
  Object.defineProperty(navigator, "clipboard", {
    value: undefined,
    configurable: true,
    writable: true,
  })
})

afterEach(() => {
  cleanup()
  if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard)
  else delete (navigator as unknown as Record<string, unknown>).clipboard
  delete (document as unknown as Record<string, unknown>).execCommand
})

describe("Copyable over plain http", () => {
  it("copies through the textarea fallback and confirms it when the browser allows it", async () => {
    const execCommand = mock(() => true)
    ;(document as unknown as Record<string, unknown>).execCommand = execCommand
    const successes = spyOn(toast, "success").mockImplementation(() => 1)
    const errors = spyOn(toast, "error").mockImplementation(() => 1)

    const { getByRole } = render(<Copyable value="tenant_slug">tenant_slug</Copyable>)

    fireEvent.click(getByRole("button", { name: "Copy" }))

    await waitFor(() => expect(successes).toHaveBeenCalled())
    expect(execCommand).toHaveBeenCalledWith("copy")
    expect(successes.mock.calls[0]?.[0]).toBe("Copied “tenant_slug”")
    expect(errors).not.toHaveBeenCalled()
    successes.mockRestore()
    errors.mockRestore()
  })

  it("shows the failure toast instead of a copied confirmation when the browser refuses", async () => {
    ;(document as unknown as Record<string, unknown>).execCommand = mock(() => false)
    const successes = spyOn(toast, "success").mockImplementation(() => 1)
    const errors = spyOn(toast, "error").mockImplementation(() => 1)

    const { getByRole } = render(<Copyable value="tenant_slug">tenant_slug</Copyable>)

    fireEvent.click(getByRole("button", { name: "Copy" }))

    await waitFor(() => expect(errors).toHaveBeenCalled())
    expect(errors.mock.calls[0]?.[0]).toBe("Could not copy")
    expect(errors.mock.calls[0]?.[1]?.description).toBe(
      "The browser refused the copy. Select the text and copy it by hand.",
    )
    expect(successes).not.toHaveBeenCalled()
    successes.mockRestore()
    errors.mockRestore()
  })
})
