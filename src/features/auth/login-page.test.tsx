import { cleanup, fireEvent, render, screen } from "@testing-library/react"
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

// WEB-15: the title's face comes through `next/font/local`, which only the
// Next compiler can evaluate (it has no runtime default export). The face
// carries no behaviour under test, so the loader returns an empty class.
mock.module("next/font/local", () => ({
  default: () => ({ className: "", variable: "", style: { fontFamily: "" } }),
}))

// WEB-15 T2: the sample dashboard is a `next/dynamic` import that loads after
// the form. Here it never loads at all, which is what proves the form needs
// nothing from it: every test in this file signs in without the showcase.
mock.module("next/dynamic", () => ({ default: () => () => null }))

import { AuthProvider } from "@/features/auth/auth-provider"
import { PRODUCT_NAME } from "@/lib/product-branding"

// WEB-15: imported after the mocks, not statically. The empty
// `next/font/local` entry has no default export, and a static import links
// the page's whole module graph before `mock.module` above can stand in for
// it ("Missing 'default' export"). (The page's logos are plain `<img>`s
// through `BrandPicture` since `ADM-2`, so `next/image` needs no stub here.)
const { LoginPage } = await import("./login-page")

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
    expect(alert.textContent).toBe("Wrong username or password.")
    expect(alert.textContent).not.toContain("invalid_or_expired")
  })

  it("marks both credential fields required and explains the marker once (UX-1)", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    const { container } = render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    // findByLabelText proves the label still associates with its control
    // once the decorative star is nested inside the label.
    const email = await screen.findByLabelText("Username or email")
    const password = screen.getByLabelText("Password")
    // Native `required` stays on the controls: noValidate means it conveys
    // semantics without changing submission behaviour.
    expect(email.hasAttribute("required")).toBe(true)
    expect(password.hasAttribute("required")).toBe(true)
    // Exactly two labels opt in, each with an aria-hidden star.
    const marked = container.querySelectorAll("label[data-required]")
    expect(marked.length).toBe(2)
    for (const label of marked) {
      const star = label.querySelector("span[aria-hidden='true']")
      expect(star?.className).toContain("before:content-['*']")
    }
    // The convention is stated once, not per field.
    expect(screen.getByText(/Required/)).toBeDefined()
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

    // Owner decision 2026-11-18 (`docs/core/features/login-page.md`): the
    // title names the workspace, and so does the copyright line.
    expect(await screen.findByRole("heading", { level: 1, name: "Sign in to Acme Data" })).toBeTruthy()
    expect(screen.getByText(`© ${new Date().getFullYear()} Acme Data`)).toBeTruthy()
    // The product's own name is nowhere in the title or the copyright: a
    // workspace that set a name must not still be called by ours. (The
    // "Powered by" byline is the one place the product keeps, decision 8.)
    expect(screen.queryByRole("heading", { name: `Sign in to ${PRODUCT_NAME}` })).toBeNull()
    expect(screen.queryByText(`© ${new Date().getFullYear()} ${PRODUCT_NAME}`)).toBeNull()

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

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in to Acme Data" })).toBeTruthy()
    const shown = pictures(container)
    // The second picture is the dark one: with no dark logo set, it shows
    // the workspace's light logo rather than the product's own dark file.
    expect(shown[1].getAttribute("src")).toBe("/api/branding/images/logo?v=v1")
  })

  it("falls back to the product's own name and logos when the workspace set none", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    const { container } = renderLoginPage()

    // The fallback is `PRODUCT_NAME`, whatever the product calls itself.
    expect(await screen.findByRole("heading", { level: 1, name: `Sign in to ${PRODUCT_NAME}` })).toBeTruthy()
    expect(screen.getByText(`© ${new Date().getFullYear()} ${PRODUCT_NAME}`)).toBeTruthy()
    // The official mark (`WEB-15`), at its own ratio.
    expect(pictures(container).map((p) => p.getAttribute("src"))).toEqual([
      "/logo-light.svg",
      "/logo-dark.svg",
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

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in to <b>x</b>" })).toBeTruthy()
    // The name is text, never HTML: a workspace that typed markup shows
    // the markup, it does not run it.
    expect(container.querySelector("b")).toBeNull()
  })

  it("keeps the 'Powered by RantAI Lakehouse' line under the form for a workspace with its own name", async () => {
    // ADM-2 decision 8: a workspace that set its own name still says who
    // built the console.
    stubLoginApi(
      401,
      { error: "invalid_or_expired" },
      {},
      { workspaceName: "Acme Data", logo: null, logoDark: null, icon: null },
    )
    renderLoginPage()

    expect(await screen.findByText("Powered by RantAI Lakehouse")).toBeTruthy()
    expect(screen.getByText(`© ${new Date().getFullYear()} Acme Data`)).toBeTruthy()
  })

  it("names the product once in the footer when the workspace set no name", async () => {
    // Planner review of the #536 merge (2026-11-18), DESIGN.md: one piece of
    // information is written once. With no name of its own, the copyright
    // already names the product, so the byline does not repeat it.
    stubLoginApi(401, { error: "invalid_or_expired" })
    renderLoginPage()

    const copyright = await screen.findByText(`© ${new Date().getFullYear()} ${PRODUCT_NAME}`)
    // The name is the product's own before the branding answer and after it
    // (the answer sets no name); waiting for the form lets that answer land
    // before the check, so the byline is checked on the settled page.
    await screen.findByLabelText("Username or email")
    expect(screen.queryByText("Powered by RantAI Lakehouse")).toBeNull()
    expect(copyright.parentElement?.textContent).not.toContain("·")
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

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in to Acme Data" })).toBeTruthy()
    const shown = pictures(container)
    fireEvent.error(shown[0])
    // A broken picture must never be left on the page: the product's own
    // file stands in for it.
    expect(shown[0].getAttribute("src")).toBe("/logo-light.svg")
  })
})

/**
 * A `keydown` whose `getModifierState("CapsLock")` answers `on`. happy-dom's
 * `KeyboardEvent` reports no lock keys at all, and React's synthetic event
 * asks the native one, so the answer is set on the instance.
 */
function keyWithCapsLock(type: "keydown" | "keyup", on: boolean): KeyboardEvent {
  const event = new KeyboardEvent(type, { key: "A", bubbles: true })
  Object.defineProperty(event, "getModifierState", {
    value: (key: string) => (key === "CapsLock" ? on : false),
  })
  return event
}

describe("LoginPage form (WEB-15 T1)", () => {
  it("opens the refusal in place, shakes the form once and selects the password for retyping", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "wrong-password")

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe("Wrong username or password.")
    expect(alert.id).toBe("login-error")

    // WEB-15 spec: "the password is selected for retyping".
    const password = screen.getByLabelText("Password") as HTMLInputElement
    expect(password.getAttribute("aria-describedby")).toBe("login-error")
    expect(document.activeElement).toBe(password)
    expect(password.selectionStart).toBe(0)
    expect(password.selectionEnd).toBe("wrong-password".length)

    // The 240 ms shake is a class on the form (the motion itself is CSS, and
    // sits inside `prefers-reduced-motion: no-preference`).
    expect(password.closest("form")?.className).toContain("login-shake")
  })

  it("folds the refusal away once the person types again", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "wrong-password")
    await screen.findByRole("alert")

    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "right-password" } })
    expect(screen.queryByRole("alert")).toBeNull()
    expect(screen.getByLabelText("Password").getAttribute("aria-invalid")).toBe("false")
  })

  it("warns while Caps Lock is on in the password field, and stops on blur", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await screen.findByLabelText("Username or email")
    const password = screen.getByLabelText("Password")

    fireEvent(password, keyWithCapsLock("keydown", true))
    expect((await screen.findByRole("alert")).textContent).toBe("Caps Lock is on")

    fireEvent(password, keyWithCapsLock("keyup", false))
    expect(screen.queryByRole("alert")).toBeNull()

    fireEvent(password, keyWithCapsLock("keydown", true))
    expect(screen.getByRole("alert").textContent).toBe("Caps Lock is on")
    fireEvent.blur(password)
    expect(screen.queryByRole("alert")).toBeNull()
  })

  it("names the eye after what it does and switches the field's type", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await screen.findByLabelText("Username or email")
    const password = screen.getByLabelText("Password") as HTMLInputElement
    expect(password.type).toBe("password")

    fireEvent.click(screen.getByRole("button", { name: "Show password" }))
    expect(password.type).toBe("text")
    fireEvent.click(screen.getByRole("button", { name: "Hide password" }))
    expect(password.type).toBe("password")
  })
})

describe("LoginPage without the sample dashboard (WEB-15 T2)", () => {
  it("renders a usable form while the sample dashboard has not loaded", async () => {
    stubLoginApi(401, { error: "invalid_or_expired" })
    render(
      <AuthProvider>
        <LoginPage />
      </AuthProvider>,
    )
    await submitCredentials("rina@meridian.example", "wrong-password")
    expect((await screen.findByRole("alert")).textContent).toBe("Wrong username or password.")

    // The panel and its caption are there; the dashboard inside is not.
    const panel = screen.getByRole("region", { name: "A preview of the console (sample data)" })
    expect(panel.textContent).toContain("Every dataset, pipeline and dashboard in one place.")
    expect(panel.querySelector("[data-motion]")).toBeNull()
  })
})
