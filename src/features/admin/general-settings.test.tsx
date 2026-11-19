import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { GeneralSettings } from "./general-settings"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** What `GET /api/settings/general` answers: a name and a logo, no icon. */
const SAVED = {
  workspaceName: "Contoh Workspace",
  logo: { set: true, type: "image/png", bytes: 1024 },
  logoDark: { set: false, type: null, bytes: null },
  icon: { set: false, type: null, bytes: null },
  updatedAt: "2026-11-16T09:00:00Z",
}

/** What the public `GET /api/branding` answers: the name and the logo's version. */
const BRANDING = {
  workspaceName: "Contoh Workspace",
  logo: { type: "image/png", version: "v1" },
  logoDark: null,
  icon: null,
}

type Call = { method: string; url: string }

/**
 * Answers the two reads, and lets a test take over one call of its own.
 * Every call is recorded, so a test can see that nothing was sent yet.
 */
function stubApi(answer?: (call: Call) => Response | null) {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, url })
    const own = answer?.({ method, url })
    if (own) return own
    if (url.includes("/api/settings/general")) return json(SAVED)
    if (url.includes("/api/branding")) return json(BRANDING)
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch
  return calls
}

describe("GeneralSettings", () => {
  it("shows the workspace's name as it is stored", async () => {
    stubApi()
    render(<GeneralSettings />)
    const input = (await screen.findByLabelText("Workspace name")) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe("Contoh Workspace"))
  })

  it("shows the server's refusal sentence as it is when a picture is refused", async () => {
    stubApi((call) => {
      if (call.method === "PUT" && call.url.includes("/api/settings/general/images/logo")) {
        return json({ error: "The logo must be at most 512 KB." }, 422)
      }
      return null
    })
    render(<GeneralSettings />)
    const file = await screen.findByLabelText("Logo")
    fireEvent.change(file, {
      target: { files: [new File(["bytes"], "logo.png", { type: "image/png" })] },
    })
    expect(await screen.findByText("The logo must be at most 512 KB.")).toBeTruthy()
  })

  it("asks before a reset and sends nothing until the reset is confirmed", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    await screen.findByLabelText("Workspace name")
    fireEvent.click(screen.getByRole("button", { name: "Reset" }))
    // The dialog asks first; nothing has been sent.
    expect(await screen.findByText("Reset the workspace settings")).toBeTruthy()
    expect(calls.some((call) => call.method === "DELETE")).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "Reset everything" }))
    await waitFor(() =>
      expect(
        calls.some((call) => call.method === "DELETE" && call.url === "/api/settings/general")
      ).toBe(true)
    )
  })

  it("refuses a name of more than 60 characters in the field", async () => {
    stubApi()
    render(<GeneralSettings />)
    const input = (await screen.findByLabelText("Workspace name")) as HTMLInputElement
    expect(input.maxLength).toBe(60)
    expect(screen.getByText(/at most 60 characters/i)).toBeTruthy()
  })

  it("offers a picker and a remove button for each of the three pictures", async () => {
    stubApi()
    render(<GeneralSettings />)
    await screen.findByLabelText("Workspace name")
    // One picker per picture: the logo, the dark-mode logo, the tab icon.
    for (const label of ["Logo", "Dark-mode logo", "Browser-tab icon"]) {
      const picker = screen.getByLabelText(label) as HTMLInputElement
      expect(picker.getAttribute("type")).toBe("file")
    }
    // The preview is the public picture, with the version that busts the cache.
    const preview = screen.getByAltText("Logo") as HTMLImageElement
    expect(preview.getAttribute("src")).toBe("/api/branding/images/logo?v=v1")
    // A remove button each: the logo is set, the other two are not, so their
    // buttons are disabled.
    const logo = screen.getByRole("button", { name: "Remove the logo" }) as HTMLButtonElement
    const dark = screen.getByRole("button", {
      name: "Remove the dark-mode logo",
    }) as HTMLButtonElement
    const icon = screen.getByRole("button", {
      name: "Remove the browser-tab icon",
    }) as HTMLButtonElement
    expect(logo.disabled).toBe(false)
    expect(dark.disabled).toBe(true)
    expect(icon.disabled).toBe(true)
  })

  /** Lets a read answer a name of its own from its second call on. */
  function readsThen(later: string | null) {
    let reads = 0
    return (call: Call) => {
      if (call.method === "GET" && call.url === "/api/settings/general") {
        reads += 1
        if (reads > 1) return json({ ...SAVED, workspaceName: later })
      }
      return null
    }
  }

  it("keeps a name the person typed when a picture change makes the server answer again (adm-121)", async () => {
    const calls = stubApi(readsThen("Changed elsewhere"))
    render(<GeneralSettings />)
    const first = (await screen.findByLabelText("Workspace name")) as HTMLInputElement
    await waitFor(() => expect(first.value).toBe("Contoh Workspace"))
    fireEvent.change(first, { target: { value: "Typed name" } })
    // Storing a picture reads the settings again; the unsaved name is not
    // the server's to overwrite.
    fireEvent.change(screen.getByLabelText("Logo"), {
      target: { files: [new File(["bytes"], "logo.png", { type: "image/png" })] },
    })
    // The form is drawn again from the second answer.
    await waitFor(() =>
      expect(
        calls.filter((c) => c.method === "GET" && c.url === "/api/settings/general")
      ).toHaveLength(2)
    )
    await new Promise((resolve) => setTimeout(resolve, 50))
    const again = (await screen.findByLabelText("Workspace name")) as HTMLInputElement
    expect(again.value).toBe("Typed name")
  })

  it("shows the product's empty name again after a reset, not the name typed before it (adm-121)", async () => {
    stubApi(readsThen(null))
    render(<GeneralSettings />)
    const first = (await screen.findByLabelText("Workspace name")) as HTMLInputElement
    await waitFor(() => expect(first.value).toBe("Contoh Workspace"))
    fireEvent.change(first, { target: { value: "Typed name" } })
    fireEvent.click(screen.getByRole("button", { name: "Reset" }))
    fireEvent.click(await screen.findByRole("button", { name: "Reset everything" }))
    // The reset cleared the name on the server; the form must say so.
    await waitFor(async () => {
      const field = (await screen.findByLabelText("Workspace name")) as HTMLInputElement
      expect(field.value).toBe("")
    })
  })
})
