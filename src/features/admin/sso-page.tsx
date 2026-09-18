"use client"

import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { authService } from "@/services"

/**
 * SSO admin page (WS8 §Phase F). Reads `GET /api/auth/providers`
 * to honestly report whether OIDC is configured on this deployment — the
 * flag is a runtime read of the API process's `AuthState`/`Config` (see
 * `routes/auth.rs:731`), so this page can never drift from the backend's
 * real state the way a build-time flag (the old
 * `NEXT_PUBLIC_SSO_ENABLED`) could.
 *
 * Deliberately does NOT read `OIDC_ROLE_MAP` over HTTP. Role mapping is
 * an env var on the API process, not a backend resource; the page text
 * surfaces the literal below so a future reader searching for it lands
 * here instead of inventing a role-map endpoint (plan Deviation D4). The
 * same posture applies to `OIDC_CLIENT_SECRET` — neither secret ever
 * crosses the `/api/auth/*` wire.
 */
export function SsoPage() {
  const state = useService((signal) => authService.providers(signal), [])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Single Sign-On"
        description="OIDC configuration for this deployment."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && !state.data.oidc ? (
        <p className="text-sm text-muted-foreground">
          OIDC is not configured on this deployment.
        </p>
      ) : null}
      {state.status === "success" && state.data.oidc ? (
        <div className="flex flex-col gap-3">
          <p className="text-sm">
            Provider:{" "}
            <span className="font-medium">{state.data.providerName}</span>
          </p>
          <p className="text-sm text-muted-foreground">
            Role mapping is configured via <code>OIDC_ROLE_MAP</code> on the
            API process — see <code>README.md</code>&apos;s config table for
            the exact syntax. This page does not read it over HTTP, the same
            way it never reads <code>OIDC_CLIENT_SECRET</code>.
          </p>
          <Button className="w-fit" render={<a href="/api/auth/oidc/start" />}>
            Test sign-in
          </Button>
        </div>
      ) : null}
    </div>
  )
}