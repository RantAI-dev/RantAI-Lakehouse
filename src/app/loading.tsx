"use client"

import { usePathname } from "next/navigation"
import { ShellMainPlaceholder } from "@/components/app-shell/shell-main-placeholder"
import { isPublicPath } from "@/features/auth/auth-provider"

/**
 * Route loading fallback. It renders inside the console's `<main>` (which already
 * supplies the padding), so it draws the same page-shaped placeholder the shell
 * skeleton and the page itself draw, and nothing on public paths (`/login`, ...),
 * which have no console frame (owner QA 2026-11-15).
 */
export default function Loading() {
  const pathname = usePathname() ?? "/"
  if (isPublicPath(pathname)) return null
  return <ShellMainPlaceholder pathname={pathname} />
}
