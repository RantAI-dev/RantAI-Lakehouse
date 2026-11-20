import { cleanup, fireEvent, render, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { toast } from "sonner"

import { CopyButton } from "./copy-button"

/**
 * `UX-6`: a console opened by IP over plain http has no
 * `navigator.clipboard` (QA found it on staging). The button must go through
 * `copyText`, whose textarea + `execCommand("copy")` fallback still works
 * there, and it must not claim "Copied" when that fallback is refused.
 *
 * The clipboard API is removed for the whole file, which is exactly the
 * staging situation; `document.execCommand` stands in for the browser's
 * answer, the one part of the fallback a DOM implementation cannot give.
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

describe("CopyButton over plain http", () => {
  it("copies through the textarea fallback and shows the check when the browser allows it", async () => {
    let copiedText = ""
    const execCommand = mock((command: string) => {
      copiedText = (document.activeElement as HTMLTextAreaElement | null)?.value ?? ""
      return command === "copy"
    })
    ;(document as unknown as Record<string, unknown>).execCommand = execCommand

    const { getByRole, queryByRole } = render(<CopyButton text="s3cr3t-value" />)

    fireEvent.click(getByRole("button", { name: "Copy" }))

    await waitFor(() => expect(queryByRole("button", { name: "Copied" })).not.toBeNull())
    expect(execCommand).toHaveBeenCalledWith("copy")
    expect(copiedText).toBe("s3cr3t-value")
  })

  it("shows the failure toast and keeps the copy icon when the browser refuses", async () => {
    ;(document as unknown as Record<string, unknown>).execCommand = mock(() => false)
    const errors = spyOn(toast, "error").mockImplementation(() => 1)

    const { getByRole, queryByRole } = render(<CopyButton text="s3cr3t-value" />)

    fireEvent.click(getByRole("button", { name: "Copy" }))

    await waitFor(() => expect(errors).toHaveBeenCalled())
    expect(errors.mock.calls[0]?.[0]).toBe("Could not copy")
    expect(errors.mock.calls[0]?.[1]?.description).toBe(
      "The browser refused the copy. Select the text and copy it by hand.",
    )
    expect(queryByRole("button", { name: "Copied" })).toBeNull()
    expect(queryByRole("button", { name: "Copy" })).not.toBeNull()
    errors.mockRestore()
  })
})
