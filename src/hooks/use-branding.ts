"use client"

import * as React from "react"
import { brandingImageUrl, brandingService, type Branding } from "@/services"
import {
  PRODUCT_LOGO_DARK,
  PRODUCT_LOGO_LIGHT,
  PRODUCT_NAME,
} from "@/lib/product-branding"
import { useService } from "./use-service"

/**
 * The workspace's branding as a surface shows it (`ADM-2`): the name, and
 * the URL of each logo. Every field the workspace left unset falls back to
 * the product's own, so a surface is never blank and never shows a broken
 * picture.
 */
export type BrandingView = {
  /** The workspace's name, or the product's own name when none is set. */
  name: string
  /** The logo for a light background. */
  logoSrc: string
  /** The logo for a dark background. */
  logoDarkSrc: string
}

/**
 * The workspace's branding with the product's own defaults filled in.
 *
 * `ADM-2-AC3`: a workspace that set only a light logo shows that logo in
 * dark mode too; the product's own dark file appears only when no logo at
 * all is set.
 */
export function resolveBranding(branding: Branding | null): BrandingView {
  const name = branding?.workspaceName ? branding.workspaceName : PRODUCT_NAME
  const logo = branding?.logo ?? null
  const logoDark = branding?.logoDark ?? null
  return {
    name,
    logoSrc: logo ? brandingImageUrl("logo", logo.version) : PRODUCT_LOGO_LIGHT,
    logoDarkSrc: logoDark
      ? brandingImageUrl("logo-dark", logoDark.version)
      : logo
        ? brandingImageUrl("logo", logo.version)
        : PRODUCT_LOGO_DARK,
  }
}

/**
 * Reads the workspace's branding for a surface (`ADM-2`, plan decision 1).
 *
 * The route is public, so the sign-in page reads it before any session
 * exists. A failure is not an error a surface has to show: the product's
 * own name and logos stand in, so the console is never blank.
 */
export function useBranding(): BrandingView {
  const state = useService((signal) => brandingService.get(signal), [])
  const branding = state.status === "success" ? state.data : null
  return React.useMemo(() => resolveBranding(branding), [branding])
}
