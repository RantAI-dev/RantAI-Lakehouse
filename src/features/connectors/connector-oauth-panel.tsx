"use client"

import * as React from "react"
import { SectionCard } from "@/components/patterns/section-card"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { connectorService } from "@/services"
import type { OAuthSignOut } from "@/services/contracts/connectors"

/**
 * Whether a connector's ingest spec says it signs in with OAuth authorization
 * code (`SRC-12` part C), and if so whether its dial names a revocation
 * address. `null` for every other connector, so the page shows no Sign in panel.
 */
export function oauthSignIn(spec: { adapter: string | null; dial: unknown }): { revocationConfigured: boolean } | null {
  if (spec.adapter !== "rest") return null
  const auth = (spec.dial as { auth?: { type?: unknown; revocationUrl?: unknown } } | null)?.auth
  if (auth?.type !== "oauth2_authorization_code") return null
  return { revocationConfigured: typeof auth.revocationUrl === "string" && auth.revocationUrl !== "" }
}

/** How often the panel asks whether the sign-in finished, while it waits. */
const POLL_MS = 2000

/**
 * What the panel says about the provider when the person signs out
 * (`SRC-12` decision 8). The API's own sentence is shown after the action;
 * this is the warning BEFORE it, from what the connector page already knows.
 */
export function signOutWarning(revocationConfigured: boolean): string {
  return revocationConfigured
    ? "Signing out deletes the stored token and tells the provider to revoke it."
    : "Signing out deletes the stored token. This connector has no revocation address, so the provider cannot be told: the token stays valid there until it expires."
}

/**
 * Whether the newest run of the connector was refused by the provider, so the
 * panel can say "sign in again" (`SRC-12` decision 7). One request for the run
 * list and, when the newest run failed, one for its log.
 */
function useSignInRefused(connectorId: string, reloadKey: number): boolean {
  const refused = useService(
    async (signal) => {
      const runs = await connectorService.listConnectorRuns(connectorId, signal)
      const newest = runs[0]
      if (!newest || newest.status !== "failed") return false
      const log = await connectorService.getConnectorRunLog(connectorId, newest.runId, signal)
      return log.attempts.some((attempt) => attempt.errorClass === "sign_in_refused")
    },
    [connectorId, reloadKey],
    { keepDataOnReload: true }
  )
  return refused.status === "success" && refused.data
}

/**
 * Sign in to a REST connector that uses OAuth authorization code (`SRC-12`
 * part C). The sign-in page opens in a new window, the provider sends the
 * person back to this product's own callback, and the API stores the refresh
 * token sealed; this panel only starts it and shows the result. It holds no
 * token and no `state`.
 *
 * The window is opened without an opener (the provider's page must not reach
 * back into this one), so the panel finds out by asking the API: it polls while
 * waiting, and asks again when the tab gets focus back.
 */
export function ConnectorOAuthPanel({
  connectorId,
  onChanged,
}: {
  connectorId: string
  /** Called after the connector signed in or out, so the page can refresh what it shows. */
  onChanged?: () => void
}) {
  const [reloadKey, setReloadKey] = React.useState(0)
  const status = useService((s) => connectorService.getOAuthStatus(connectorId, s), [connectorId, reloadKey], {
    keepDataOnReload: true,
  })
  const refused = useSignInRefused(connectorId, reloadKey)
  const start = useServiceAction((signal) => connectorService.startOAuth(connectorId, signal))
  const signOut = useServiceAction((signal) => connectorService.signOutOAuth(connectorId, signal))
  const [waiting, setWaiting] = React.useState(false)
  const [signedOut, setSignedOut] = React.useState<OAuthSignOut | null>(null)
  const [confirmingSignOut, setConfirmingSignOut] = React.useState(false)
  const wasSignedIn = React.useRef<boolean | null>(null)

  const signedIn = status.status === "success" ? status.data.signedIn : null
  const refresh = React.useCallback(() => setReloadKey((k) => k + 1), [])

  // While the person is at the provider: ask every few seconds; stop when the
  // connector is signed in that did not have a token before, or after the
  // sign-in's own ten minutes.
  React.useEffect(() => {
    if (!waiting) return
    const timer = window.setInterval(refresh, POLL_MS)
    const stop = window.setTimeout(() => setWaiting(false), 10 * 60 * 1000)
    return () => {
      window.clearInterval(timer)
      window.clearTimeout(stop)
    }
  }, [waiting, refresh])
  React.useEffect(() => {
    if (signedIn === null) return
    if (waiting && wasSignedIn.current === false && signedIn) {
      setWaiting(false)
      onChanged?.()
    }
    wasSignedIn.current = signedIn
  }, [signedIn, waiting, onChanged])
  // Back from the provider's window: ask at once.
  React.useEffect(() => {
    const onFocus = () => refresh()
    window.addEventListener("focus", onFocus)
    return () => window.removeEventListener("focus", onFocus)
  }, [refresh])

  async function handleSignIn() {
    setSignedOut(null)
    const started = await start.run()
    if (!started) return
    // No opener, no referrer: the provider's page cannot reach this window.
    window.open(started.authorizeUrl, "_blank", "noopener,noreferrer")
    setWaiting(true)
  }

  async function handleSignOut() {
    const result = await signOut.run()
    setConfirmingSignOut(false)
    setWaiting(false)
    if (result) {
      setSignedOut(result)
      refresh()
      onChanged?.()
    }
  }

  if (status.status === "loading") return <LoadingSkeleton rows={2} />
  if (status.status === "error") return <ErrorState error={status.error} onRetry={status.reload} />
  const { callbackUrl, revocationConfigured } = status.data

  return (
    <SectionCard
      title="Sign in"
      description="This connector signs in to its source with OAuth. Someone signs in once; the connector then keeps itself signed in."
    >
      <div className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          {signedIn ? <Pill tone="success">Signed in</Pill> : <Pill tone="warning">Not signed in</Pill>}
          {refused ? <Pill tone="destructive">The provider refused the last run</Pill> : null}
        </div>
        {refused ? (
          <p className="text-sm text-destructive" role="alert">
            The provider refused this connector&apos;s sign-in on its last run. Sign in again.
          </p>
        ) : null}
        {callbackUrl === null ? (
          <p className="text-sm text-destructive" role="alert">
            The operator has not set this installation&apos;s callback address (CONNECTOR_OAUTH_REDIRECT_URI), so
            signing in is not possible yet.
          </p>
        ) : null}
        {start.status === "error" ? (
          <p className="text-sm text-destructive" role="alert">
            {start.error.message}
          </p>
        ) : null}
        {waiting ? (
          <p className="text-sm text-muted-foreground" role="status">
            Waiting for the provider. Finish signing in in the new window; this page updates by itself.
          </p>
        ) : null}
        {signedOut ? (
          <p className="text-sm text-muted-foreground" role="status">
            Signed out. {signedOut.message}
          </p>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={start.status === "pending" || callbackUrl === null}
            onClick={handleSignIn}
          >
            {start.status === "pending" ? "Opening…" : signedIn ? "Sign in again" : "Sign in"}
          </Button>
          {waiting ? (
            <Button size="sm" variant="outline" onClick={refresh}>
              Check again
            </Button>
          ) : null}
          {signedIn ? (
            <Button size="sm" variant="ghost" onClick={() => setConfirmingSignOut(true)}>
              Sign out
            </Button>
          ) : null}
        </div>
        {confirmingSignOut ? (
          <div className="space-y-2 rounded-md border border-border p-3" role="group" aria-label="Confirm sign out">
            <p className="text-sm">{signOutWarning(revocationConfigured)}</p>
            <div className="flex gap-2">
              <Button size="sm" variant="destructive" disabled={signOut.status === "pending"} onClick={handleSignOut}>
                {signOut.status === "pending" ? "Signing out…" : "Sign out"}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setConfirmingSignOut(false)}>
                Cancel
              </Button>
            </div>
            {signOut.status === "error" ? (
              <p className="text-sm text-destructive" role="alert">
                {signOut.error.message}
              </p>
            ) : null}
          </div>
        ) : null}
        {callbackUrl ? (
          <div className="space-y-1" data-testid="oauth-callback">
            <p className="text-xs font-medium">Callback address</p>
            <code className="block break-all rounded-md bg-muted px-2 py-1.5 text-xs">{callbackUrl}</code>
            <p className="text-xs text-muted-foreground">
              The redirect URI to enter in the provider&apos;s app settings.
            </p>
          </div>
        ) : null}
      </div>
    </SectionCard>
  )
}
