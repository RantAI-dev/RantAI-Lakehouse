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

type Call = { method: string; url: string; body?: BodyInit | null }

/**
 * Answers the two reads, and lets a test take over one call of its own.
 * Every call is recorded, so a test can see that nothing was sent yet.
 */
function stubApi(answer?: (call: Call) => Response | null) {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, url, body: init?.body ?? null })
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

  // ── The picture is checked in the browser before it is sent (adm-120) ────

  /** A picture file of `size` bytes, without allocating them. */
  function pictureFile(name: string, type: string, size: number): File {
    const file = new File(["bytes"], name, { type })
    Object.defineProperty(file, "size", { value: size })
    return file
  }

  /** Whether a picture of this kind was sent to the API. */
  function sentPicture(calls: Call[], kind: string): boolean {
    return calls.some(
      (call) => call.method === "PUT" && call.url.includes(`/api/settings/general/images/${kind}`)
    )
  }

  it("refuses an oversized logo in the browser, with the server's sentence, and sends nothing", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Logo")
    fireEvent.change(picker, {
      target: { files: [pictureFile("logo.png", "image/png", 20 * 1024 * 1024)] },
    })
    // The server's own sentence, the one it would answer with.
    expect(await screen.findByText("The logo must be at most 512 KB.")).toBeTruthy()
    expect(sentPicture(calls, "logo")).toBe(false)
  })

  it("sends a logo the API accepts, with the file", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Logo")
    fireEvent.change(picker, {
      target: { files: [pictureFile("logo.png", "image/png", 400 * 1024)] },
    })
    await waitFor(() => expect(sentPicture(calls, "logo")).toBe(true))
    // The picture itself goes with the request.
    const put = calls.find((call) => call.method === "PUT")
    expect(put?.body).toBeInstanceOf(FormData)
    expect((put?.body as FormData).get("file")).toBeTruthy()
  })

  it("refuses a logo of the wrong format, with the server's sentence, and sends nothing", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Logo")
    fireEvent.change(picker, {
      target: { files: [pictureFile("logo.svg", "image/svg+xml", 1024)] },
    })
    expect(await screen.findByText("The logo must be a PNG, JPEG or WebP image.")).toBeTruthy()
    expect(sentPicture(calls, "logo")).toBe(false)
  })

  it("refuses a logo of the wrong type that is also too big with the type sentence", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Logo")
    fireEvent.change(picker, {
      target: { files: [pictureFile("logo.svg", "image/svg+xml", 20 * 1024 * 1024)] },
    })
    // The type is judged first, as the API judges it.
    expect(await screen.findByText("The logo must be a PNG, JPEG or WebP image.")).toBeTruthy()
    expect(sentPicture(calls, "logo")).toBe(false)
  })

  it("refuses an icon over 64 KB, with the server's sentence, and sends nothing", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Browser-tab icon")
    fireEvent.change(picker, {
      target: { files: [pictureFile("icon.png", "image/png", 65_537)] },
    })
    expect(await screen.findByText("The icon must be a PNG image of at most 64 KB.")).toBeTruthy()
    expect(sentPicture(calls, "icon")).toBe(false)
  })

  it("sends an icon the API accepts", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Browser-tab icon")
    fireEvent.change(picker, {
      target: { files: [pictureFile("icon.png", "image/png", 4096)] },
    })
    await waitFor(() => expect(sentPicture(calls, "icon")).toBe(true))
  })

  it("refuses an oversized dark-mode logo with the logo's sentence, and sends nothing", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Dark-mode logo")
    fireEvent.change(picker, {
      target: { files: [pictureFile("logo-dark.png", "image/png", 20 * 1024 * 1024)] },
    })
    // The dark-mode logo follows the logo's rules and the logo's sentences.
    expect(await screen.findByText("The logo must be at most 512 KB.")).toBeTruthy()
    expect(sentPicture(calls, "logo-dark")).toBe(false)
  })

  it("sends a file whose type the browser does not know, so the server sniffs it", async () => {
    const calls = stubApi()
    render(<GeneralSettings />)
    const picker = await screen.findByLabelText("Logo")
    fireEvent.change(picker, {
      target: { files: [pictureFile("logo.bin", "application/octet-stream", 400 * 1024)] },
    })
    // The browser knows no type, so the console refuses nothing: the API
    // reads the first bytes and stays the authority.
    await waitFor(() => expect(sentPicture(calls, "logo")).toBe(true))
  })
})
