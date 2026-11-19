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
  /**
   * The workspace's name, or the product's own name when none is set. Empty
   * while the answer is still on its way: the product's name shown for a
   * moment and then replaced by the workspace's was the flicker `adm-121`
   * removes, so a surface shows nothing until it knows.
   */
  name: string
  /** True until `GET /api/branding` has answered, with data or with an error. */
  loading: boolean
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
    loading: false,
    logoSrc: logo ? brandingImageUrl("logo", logo.version) : PRODUCT_LOGO_LIGHT,
    logoDarkSrc: logoDark
      ? brandingImageUrl("logo-dark", logoDark.version)
      : logo
        ? brandingImageUrl("logo", logo.version)
        : PRODUCT_LOGO_DARK,
  }
}

/** What stands in for the branding while the answer is on its way. */
const LOADING_VIEW: BrandingView = {
  name: "",
  loading: true,
  // The pictures stay the product's own until the answer: this change is
  // about the name. Drawing none instead would mount each `<img>` only once
  // the workspace's logo URL is known, and that is a separate decision.
  logoSrc: PRODUCT_LOGO_LIGHT,
  logoDarkSrc: PRODUCT_LOGO_DARK,
}

/** The one `GET /api/branding` answer, as the shell shares it. */
export type BrandingState = {
  status: "loading" | "success" | "error"
  branding: Branding | null
  /** Asks again, for a page that just changed the branding. */
  reload: () => void
}

/**
 * Where a surface finds the shared answer. The app shell mounts one
 * `BrandingProvider`, so the sidebar, the navbar, the palette and the
 * settings page read one request between them (`adm-121`, decision 1).
 */
export const BrandingContext = React.createContext<BrandingState | null>(null)

/**
 * Fetches the public branding once for whoever calls it. Without `enabled`
 * it sends nothing and the result is not meant to be read: `useBrandingState`
 * calls it on every render (hooks cannot be conditional) and only the
 * provider-less caller turns it on.
 */
export function useBrandingRequest(enabled: boolean): BrandingState {
  // `keepDataOnReload`: after the General page changes a picture, the name
  // and logos stay on screen while the new answer comes, instead of
  // blanking the sidebar.
  const state = useService(
    async (signal) => (enabled ? brandingService.get(signal) : null),
    [enabled],
    { keepDataOnReload: true }
  )
  const branding = state.status === "success" ? state.data : null
  const { status, reload } = state
  return React.useMemo(
    () => ({ status, branding, reload }),
    [status, branding, reload]
  )
}

/**
 * The branding answer a surface reads: the shell's shared one, or, for a
 * page outside the shell (the sign-in page, a public link, an embed), its
 * own single request.
 */
export function useBrandingState(): BrandingState {
  const shared = React.useContext(BrandingContext)
  const own = useBrandingRequest(shared === null)
  return shared ?? own
}

/**
 * Reads the workspace's branding for a surface (`ADM-2`, plan decision 1).
 *
 * The route is public, so the sign-in page reads it before any session
 * exists. A failure is not an error a surface has to show: the product's
 * own name and logos stand in, so the console is never blank. While the
 * answer is still on its way the view is empty (`loading`), not the
 * product's own.
 */
export function useBranding(): BrandingView {
  const { status, branding } = useBrandingState()
  return React.useMemo(
    () => (status === "loading" ? LOADING_VIEW : resolveBranding(branding)),
    [status, branding]
  )
}
