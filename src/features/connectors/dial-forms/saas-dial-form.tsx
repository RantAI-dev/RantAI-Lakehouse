"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { saasVendorForType, type SaasVendor } from "@/lib/connectors/saas-vendors"
import type { SaasDial } from "@/services/contracts/connectors"

type SignIn = "browser" | "integration"

/**
 * `dial` for the `saas` adapter (`SRC-3`): the vendor is the connector type the
 * person picked, so this form asks for only two things, the one part of the
 * vendor's address that is theirs (`account`) and how the connector signs in.
 * It renders no address and no secret field: the vendor's addresses come from
 * the vendor table on the server (AD2, AD8) and the credentials are the
 * connector's own credential slots, entered elsewhere in the wizard.
 */
export function SaasDialForm({
  typeName,
  value,
  onChange,
  oauthCallbackUrl,
}: {
  typeName: string | null
  value: SaasDial | null
  onChange: (next: SaasDial) => void
  /** This product's OAuth callback address, shown with the browser sign-in (`SRC-12`). `undefined` while unknown. */
  oauthCallbackUrl?: string | null
}) {
  const vendor = saasVendorForType(typeName)
  if (!vendor) {
    return (
      <p className="text-sm text-muted-foreground">
        This build has no connection form for this business app.
      </p>
    )
  }
  const dial: SaasDial = value ?? {
    vendor: vendor.id,
    account: "",
    auth: { type: "oauth2_authorization_code", clientId: "" },
  }
  const signIn: SignIn = dial.auth.type === "integration" ? "integration" : "browser"
  function setSignIn(next: SignIn) {
    onChange({
      ...dial,
      vendor: (vendor as SaasVendor).id,
      auth: next === "integration" ? { type: "integration" } : { type: "oauth2_authorization_code", clientId: "" },
    })
  }

  return (
    <div className="grid gap-3">
      <div className="space-y-1.5">
        <Label htmlFor="saas-dial-account">{vendor.accountLabel}</Label>
        <Input
          id="saas-dial-account"
          value={dial.account}
          onChange={(e) => onChange({ ...dial, vendor: vendor.id, account: e.target.value })}
          autoComplete="off"
        />
        <p className="text-xs text-muted-foreground">
          Only this part of the address is yours to enter. The rest is fixed for {vendor.label} and cannot be
          changed here.
        </p>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="saas-dial-sign-in">How it signs in</Label>
        <select
          id="saas-dial-sign-in"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={signIn}
          onChange={(e) => setSignIn(e.target.value as SignIn)}
        >
          <option value="browser">Sign in to {vendor.label} in a browser window</option>
          {vendor.integration ? <option value="integration">An integration credential</option> : null}
        </select>
        {!vendor.integration ? (
          <p className="text-xs text-muted-foreground">
            {vendor.label} offers no integration credential that this product accepts, so the connector signs in
            in the browser.
          </p>
        ) : null}
      </div>

      {dial.auth.type === "oauth2_authorization_code" ? (
        <>
          <div className="space-y-1.5">
            <Label htmlFor="saas-dial-client-id">Client ID</Label>
            <Input
              id="saas-dial-client-id"
              value={dial.auth.clientId}
              onChange={(e) =>
                onChange({ ...dial, vendor: vendor.id, auth: { type: "oauth2_authorization_code", clientId: e.target.value } })
              }
              autoComplete="off"
            />
          </div>
          <div className="space-y-1" data-testid="oauth-callback">
            <p className="text-sm font-medium">Callback address</p>
            {oauthCallbackUrl ? (
              <>
                <code className="block break-all rounded-md bg-muted px-2 py-1.5 text-xs">{oauthCallbackUrl}</code>
                <p className="text-xs text-muted-foreground">
                  Enter this address in {vendor.label}&apos;s app settings as the redirect URI. Signing in is done
                  from the connector page, once the connector is saved.
                </p>
              </>
            ) : oauthCallbackUrl === null ? (
              <p className="text-xs text-destructive">
                The operator has not set this installation&apos;s callback address (CONNECTOR_OAUTH_REDIRECT_URI),
                so signing in is not possible yet.
              </p>
            ) : null}
          </div>
        </>
      ) : null}
    </div>
  )
}
