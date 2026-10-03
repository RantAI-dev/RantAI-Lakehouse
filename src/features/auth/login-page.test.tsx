import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"

// `LoginPage` calls `useRouter()` at the top of the component, which
// throws (or expects a Next.js AppRouter context) in a bare happy-dom
// environment, so stub `next/navigation` at the module boundary before
// any import resolves. `mock.module` is hoisted to the top of the file
// by bun's test runner, so the stub is in place before the `import`
// statements below pull in the page (same pattern as
// `src/components/app-shell/tenant-switcher.test.tsx`).
mock.module("next/navigation", () => ({
  usePathname: () => "/login",
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

import { AuthProvider } from "@/features/auth/auth-provider"
import { LoginPage } from "./login-page"

const originalFetch = global.fetch

afterEach(() => {
  // Same non-auto-cleanup rationale as sessions-page.test.tsx: without
  // this the previous `it`'s page would still be in the DOM and
  // `findByLabelText` would find two email fields.
  cleanup()
  global.fetch = originalFetch
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
  fireEvent.change(await screen.findByLabelText("Email"), { target: { value: email } })
  fireEvent.change(screen.getByLabelText("Password"), { target: { value: password } })
  fireEvent.click(screen.getByRole("button", { name: /sign in/i }))
}

describe("LoginPage lockout surface (plan T5)", () => {
  it("shows the server's lockout message and the wait in whole minutes on a 429", async () => {
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
    expect(alert.textContent).toContain("Too many failed sign-in attempts. Try again later.")
    // 300 s renders as the whole-minute wait, pluralised properly
    // ("minutes", not "minute(s)" — review SHOULD-FIX 6, 2026-10-03).
    expect(alert.textContent).toContain("Try again in about 5 minutes.")
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
    expect(alert.textContent).toBe("Invalid email or password.")
    expect(alert.textContent).not.toContain("invalid_or_expired")
  })
})
