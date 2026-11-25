"use client"

import * as React from "react"

import { settingsService, type DeploymentSettings } from "@/services"

/**
 * The installation summary for the console chrome (`BI-44`), when the
 * caller is allowed to read it.
 *
 * `GET /api/settings/deployment` is `settings:write`-gated in
 * `rust/crates/lakehouse-api/src/policy.rs`. Fetching it on every page for
 * every signed-in person would fire a 403 for each non-admin role
 * (`WEB-11` / `ADM-13`), so the caller passes the permission decision in
 * and this hook does nothing at all for anyone without it. A failed or
 * slow answer leaves `null`: callers keep the full-mode view rather than
 * guessing an installation mode from a health probe. The service goes
 * through `apiFetch`, so the session and tenant handling stay in one place.
 */
export function useDeploymentSettings(canReadDeployment: boolean): DeploymentSettings | null {
  const [deployment, setDeployment] = React.useState<DeploymentSettings | null>(null)

  React.useEffect(() => {
    if (!canReadDeployment) return
    const controller = new AbortController()
    settingsService
      .getDeployment(controller.signal)
      .then((answer) => {
        if (!controller.signal.aborted) setDeployment(answer)
      })
      .catch(() => {
        // An admin whose request failed is still an admin; the console has
        // no honest answer about the mode, so it shows the unchanged view.
      })
    return () => controller.abort()
  }, [canReadDeployment])

  return deployment
}
