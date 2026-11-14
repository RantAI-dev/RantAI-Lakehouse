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

// The page header renders two `next/image` logos; `getImgProps` builds
// an absolute loader URL that happy-dom's URL parser rejects for a
// root-relative src. The images carry no behavior under test, so render
// them as plain nothing at the module boundary.
mock.module("next/image", () => ({ default: () => null }))

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

// WEB-15: imported after the mocks, not statically. The empty
// `next/font/local` entry has no default export, and a static import links
// the page's whole module graph before `mock.module` above can stand in for
// it ("Missing 'default' export"); `next/image` never hit this because its
// real module does have one.
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
 * Stubs the two routes a failed login touches: `GET /api/auth/me` (the
 * provider's mount-time session check — a 401 means "no live session",
 * which is what makes the form render instead of the skeleton) and
 * `POST /api/auth/login`, which answers `status`/`body`/`headers`.
 */
function stubLoginApi(status: number, body: unknown, headers: Record<string, string> = {}): void {
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
