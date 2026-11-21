"use client"

import type { ComponentProps } from "react"
import { usePathname } from "next/navigation"
import { ThemeProvider } from "@rantai/design-system/components/theme-provider"

/**
 * The console's theme provider (`RPT-3`, PR #641 review SHOULD-FIX 2).
 *
 * The print views — `/render/dashboard/<token>`, `/render/chart/<token>` and
 * `/render/report/<token>` (BI-18 part D) — are opened by a headless browser
 * that makes the PDF. They are always light: with the design system's default
 * (dark) the page gets `color-scheme: dark` on `<html>`, and the browser then
 * paints the whole PDF page, margin included, with the dark canvas colour
 * (#121212) before the white page is drawn. Every other route keeps the
 * default and the person's own choice.
 */
export function isPrintViewPath(pathname: string | null): boolean {
  return pathname === "/render" || (pathname?.startsWith("/render/") ?? false)
}

export function ConsoleThemeProvider(props: ComponentProps<typeof ThemeProvider>) {
  const pathname = usePathname()
  return (
    <ThemeProvider
      {...props}
      forcedTheme={isPrintViewPath(pathname) ? "light" : props.forcedTheme}
    />
  )
}
