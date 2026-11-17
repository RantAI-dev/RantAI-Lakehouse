"use client"

import * as React from "react"
import { usePathname, useRouter } from "next/navigation"
import { useAuth } from "@/features/auth/auth-provider"
import { canOpen, NAV_GROUPS } from "./nav-config"
import { PermissionState } from "@/components/patterns/page-states"

/**
 * ADM-13 (D8): Route guard mounted in AppFrame around the page content.
 * When `canOpen` is false, renders `PermissionState` so the page component
 * never mounts and makes no data/API requests.
 *
 * Exception: `/` for a user without an assistant permission redirects to the
 * first `canOpen` nav item. A Dashboard Viewer lands on `/dashboards`.
 * A user with nothing open gets `PermissionState`.
 */
export function RouteGuard({ children }: { children: React.ReactNode }) {
  const pathname = usePathname() ?? "/"
  const router = useRouter()
  const { hasPermission } = useAuth()

  const hasAssistant =
    hasPermission("assistant:use") || hasPermission("assistant:ask")

  const firstAllowedHref = React.useMemo(() => {
    for (const group of NAV_GROUPS) {
      for (const item of group.items) {
        if (item.href !== "/" && canOpen(item.href, hasPermission)) {
          return item.href
        }
      }
    }
    return null
  }, [hasPermission])

  React.useEffect(() => {
    if (pathname === "/" && !hasAssistant && firstAllowedHref) {
      router.replace(firstAllowedHref)
    }
  }, [pathname, hasAssistant, firstAllowedHref, router])

  // Exception for Home ('/'):
  if (pathname === "/") {
    if (hasAssistant) {
      return <>{children}</>
    }
    if (firstAllowedHref) {
      return null
    }
    return <PermissionState />
  }

  // All other pages: check canOpen
  if (!canOpen(pathname, hasPermission)) {
    return <PermissionState />
  }

  return <>{children}</>
}
