"use client"

import * as React from "react"
import { Moon, Sun } from "lucide-react"
import { useTheme } from "next-themes"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

/**
 * Toggles between light and dark themes via next-themes.
 * Renders a theme-agnostic placeholder until mounted to avoid hydration mismatch.
 */
export function ThemeToggle({ className }: { className?: string }) {
  const { resolvedTheme, setTheme } = useTheme()
  const [mounted, setMounted] = React.useState(false)

  React.useEffect(() => {
    setMounted(true)
  }, [])

  React.useEffect(() => {
    if (!mounted) return
    const meta = document.querySelector('meta[name="theme-color"]')
    if (meta) {
      meta.setAttribute(
        "content",
        resolvedTheme === "dark" ? "#050A30" : "#ffffff"
      )
    }
  }, [mounted, resolvedTheme])

  const isDark = mounted && resolvedTheme === "dark"

  return (
    <Button
      variant="ghost"
      size="icon"
      className={cn(
        "size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground",
        className
      )}
      aria-label="Toggle color theme"
      disabled={!mounted}
      onClick={() => setTheme(isDark ? "light" : "dark")}
    >
      {isDark ? <Sun className="size-4" /> : <Moon className="size-4" />}
    </Button>
  )
}
