import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"

// `LoginPage` calls `useRouter()` at the top of the component, which
// throws (or expects a Next.js AppRouter context) in a bare happy-dom
// environment, so stub `next/navigation` at the module boundary before
// any import resolves. `mock.module` is hoisted to the top of the file
// by bun's test runner, so the stub is in place before the `import`
// statements below pull in the page (same pattern as
// `src/components/app-shell/tenant-switcher.test.tsx`).
// `SEC-65` D2: `LoginPage` also renders `SsoNotice`, which reads
// `useSearchParams()`; each test sets `search` before rendering.
let search = new URLSearchParams()

mock.module("next/navigation", () => ({
  usePathname: () => "/login",
  useSearchParams: () => search,
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { AuthProvider } from "@/features/auth/auth-provider"
import { LoginPage } from "./login-page"

const originalFetch = global.fetch

afterEach(() => {
  // Same non-auto-cleanup rationale as sessions-page.test.tsx: without
  // this the previous `it`'s page would still be in the DOM and
  // `findByLabelText` would find two email fields.
  cleanup()
  global.fetch = originalFetch
  search = new URLSearchParams()
})

/**
 * Stubs the three routes a failed login touches: `GET /api/auth/me` (the
 * provider's mount-time session check — a 401 means "no live session",
 * which is what makes the form render instead of the skeleton),
 * `POST /api/auth/login`, which answers `status`/`body`/`headers`, and
 * the public `GET /api/branding`, which says the workspace's own name
 * and pictures.
 */
function stubLoginApi(
  status: number,
  body: unknown,
  headers: Record<string, string> = {},
  branding: unknown = { workspaceName: null, logo: null, logoDark: null, icon: null },
): void {
  global.fetch = mock(
    async (input: RequestInfo | URL) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      if (url.includes("/api/auth/me")) {
        return new Response(null, { status: 401 })
      }
      if (url.includes("/api/auth/login")) {
        return new Response(JSON.stringify(body), {
          status,
          headers: { "Content-Type": "application/json", ...headers },
        })
      }
      if (url.includes("/api/branding")) {
        return new Response(JSON.stringify(branding), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
      }
      return new Response("{}", { status: 200 })
    },
  ) as unknown as typeof fetch
}

async function submitCredentials(email: string, password: string): Promise<void> {
  // The form only renders once the mount-time session check resolved;
  // awaiting the email field is what makes these tests deterministic.
  fireEvent.change(await screen.findByLabelText("Username or email"), { target: { value: email } })
  fireEvent.change(screen.getByLabelText("Password"), { target: { value: password } })
  fireEvent.click(screen.getByRole("button", { name: /sign in/i }))
}

describe("LoginPage lockout surface (plan T5)", () => {
  it("shows one sentence with the wait in whole minutes on a 429", async () => {
    stubLoginApi(
      429,
      { error: "Too many failed sign-in attempts. Try again later." },
      { "Retry-After": "300" },
    )
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "whatever")

    const alert = await screen.findByRole("alert")
    // QA LH-SEC-2-02 (ADM-14 D13): the console owns the whole sentence and
    // does not repeat the server's "Try again later." after it. This case
    // asserted both sentences until then, which was the defect.
    expect(alert.textContent).toBe(
      "Too many failed sign-in attempts: try again in about 5 minutes.",
    )
    expect(alert.textContent).not.toContain("Try again later")
  })

  it("shows the server's sentence when a 429 carries no wait", async () => {
    stubLoginApi(429, { error: "Too many failed sign-in attempts. Try again later." })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "whatever")

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe("Too many failed sign-in attempts. Try again later.")
  })

  it("says about one minute for a wait of 60 seconds", async () => {
    stubLoginApi(
      429,
      { error: "Too many failed sign-in attempts. Try again later." },
      { "Retry-After": "60" },
    )
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "whatever")

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe(
      "Too many failed sign-in attempts: try again in about 1 minute.",
    )
  })

  it("shows the SEC-65 refusal sentence the callback redirects with", async () => {
    // `SEC-65` D2: the API sends a refused single sign-on here with
    // `ssoNotice=email_not_verified`; the sentence is the console's.
    search = new URLSearchParams("ssoNotice=email_not_verified")
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )

    const notice = await screen.findByRole("status")
    expect(notice.textContent).toContain(
      "Your identity provider did not confirm your email address, so this sign-in cannot be joined to your existing account.",
    )
  })

  it("still shows the credentials message on a 401", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "wrong-password")

    const alert = await screen.findByRole("alert")
    // One generic message regardless of cause — the page must not leak
    // the server's non-enumeration constant ("invalid_or_expired").
    expect(alert.textContent).toBe("Invalid username/email or password.")
    expect(alert.textContent).not.toContain("invalid_or_expired")
  })

  // UX-4: both fields are required and there is nothing optional on the
  // sign-in form, so it shows no star and no "* Required" line; the controls
  // keep their native `required` semantics (UX-1-R10, unchanged).
  it("keeps both credential fields required without any star or legend (UX-4)", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    const { container } = render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    const email = await screen.findByLabelText("Username or email")
    const password = screen.getByLabelText("Password")
    // Native `required` stays on the controls: noValidate means it conveys
    // semantics without changing submission behaviour.
    expect(email.hasAttribute("required")).toBe(true)
    expect(password.hasAttribute("required")).toBe(true)
    // Every input is required, so no label opts in to a star.
    expect(container.querySelectorAll("label[data-required]").length).toBe(0)
    expect(screen.queryByText(/Required/)).toBeNull()
  })
})

describe("the sign-in page shows the workspace's own branding (ADM-2)", () => {
  /** The header's two pictures, in the order they are drawn: light, then dark. */
  function pictures(container: HTMLElement): HTMLImageElement[] {
    return Array.from(container.querySelectorAll("img"))
  }

  function renderLoginPage() {
    return render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
  }

  it("shows the workspace name and logo from settings, not a fixed product name", async () => {
    stubLoginApi(
      401,
      { error: "invalid_or_expired" },
      {},
      {
        workspaceName: "Acme Data",
        logo: { type: "image/png", version: "v1" },
        logoDark: { type: "image/png", version: "v2" },
        icon: null,
      },
    )
    const { container } = renderLoginPage()

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    // The product's own name is nowhere on the page: a workspace that set
    // a name must not still be called RantAI Lakehouse.
    expect(screen.queryByText("RantAI Lakehouse")).toBeNull()

    const shown = pictures(container)
    expect(shown.map((p) => p.getAttribute("src"))).toEqual([
      "/api/branding/images/logo?v=v1",
      "/api/branding/images/logo-dark?v=v2",
    ])
    expect(shown.map((p) => p.getAttribute("alt"))).toEqual(["Acme Data", "Acme Data"])
  })

  it("shows the workspace's light logo in dark mode too when no dark logo is set (ADM-2-AC3)", async () => {
    stubLoginApi(
      401,
      { error: "invalid_or_expired" },
      {},
      {
        workspaceName: "Acme Data",
        logo: { type: "image/png", version: "v1" },
        logoDark: null,
        icon: null,
      },
    )
    const { container } = renderLoginPage()

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    const shown = pictures(container)
    // The second picture is the dark one: with no dark logo set, it shows
    // the workspace's light logo rather than the product's own dark file.
    expect(shown[1].getAttribute("src")).toBe("/api/branding/images/logo?v=v1")
  })

  it("falls back to the product's own name and logos when the workspace set none", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    const { container } = renderLoginPage()

    await waitFor(() => expect(screen.getByText("RantAI Lakehouse")).toBeTruthy())
    expect(pictures(container).map((p) => p.getAttribute("src"))).toEqual([
      "/logo-light.png",
      "/logo-dark.png",
    ])
  })

  it("shows a workspace name that holds markup as plain text", async () => {
    stubLoginApi(
      401,
      { error: "invalid_or_expired" },
      {},
      { workspaceName: "<b>x</b>", logo: null, logoDark: null, icon: null },
    )
    const { container } = renderLoginPage()

    expect(await screen.findByText("<b>x</b>")).toBeTruthy()
    // The name is text, never HTML: a workspace that typed markup shows
    // the markup, it does not run it.
    expect(container.querySelector("b")).toBeNull()
  })

  it("keeps the 'Powered by RantAI Lakehouse' line under the form", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    renderLoginPage()

    expect(await screen.findByText("Powered by RantAI Lakehouse")).toBeTruthy()
  })

  it("falls back to the product's own logo when the workspace's logo fails to load", async () => {
    stubLoginApi(
      401,
      { error: "invalid_or_expired" },
      {},
      {
        workspaceName: "Acme Data",
        logo: { type: "image/png", version: "v1" },
        logoDark: null,
        icon: null,
      },
    )
    const { container } = renderLoginPage()

    expect(await screen.findByText("Acme Data")).toBeTruthy()
    const shown = pictures(container)
    fireEvent.error(shown[0])
    // A broken picture must never be left on the page: the product's own
    // file stands in for it.
    expect(shown[0].getAttribute("src")).toBe("/logo-light.png")
  })
})
