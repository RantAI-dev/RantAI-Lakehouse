"use client"

import * as React from "react"
import { BrandingContext, useBrandingRequest } from "@/hooks/use-branding"

/**
 * Reads `GET /api/branding` once for the console's shell (`adm-121`,
 * decision 1). The sidebar, the navbar, the command palette and the General
 * settings page all show the workspace's name or pictures; without this each
 * of them asked on its own, so a page cost four requests and each answer
 * landed at its own moment.
 *
 * A page outside the shell (sign-in, a public link, an embed) has no
 * provider above it, and `useBranding` makes its one request there.
 */
export function BrandingProvider({ children }: { children: React.ReactNode }) {
  const value = useBrandingRequest(true)
  return <BrandingContext.Provider value={value}>{children}</BrandingContext.Provider>
}
