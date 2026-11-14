"use client"

import * as React from "react"
import Link from "next/link"
import { CircleUserRound, KeyRound, LogOut, Monitor, Moon, Sun } from "lucide-react"
import { useTheme } from "next-themes"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { CONSOLE_VERSION } from "@/features/version/console-version"

/** Sun until mounted, so the server render and the first client render agree. */
function ThemeIcon({
  mounted,
  theme,
  resolvedTheme,
}: Readonly<{
  mounted: boolean
  theme: string | undefined
  resolvedTheme: string | undefined
}>) {
  if (mounted && theme === "system") return <Monitor className="size-4" />
  if (mounted && resolvedTheme === "dark") return <Moon className="size-4" />
  return <Sun className="size-4" />
}

/**
 * Who's signed in, with a logout affordance and theme selection. Renders once `AuthProvider` has resolved a user (see `AppFrame`).
 *
 * BI-61 review round 1, SHOULD-FIX 7 (owner decision 2026-11-14): the account
 * menu lives in one place, the sidebar's footer card, which is its trigger
 * (`trigger`, shadcn's NavUser pattern); the navbar's profile button is gone.
 * Without `trigger` it draws the old icon button, which its own test uses.
 *
 * Exported for its own test: the account menu's version line (REL-2
 * S1-T5) is the one thing here that says which build the console is, and
 * rendering the whole shell around it would need providers the menu does
 * not use.
 */
export function UserMenu({
  userName,
  userEmail,
  onLogout,
  trigger,
  children,
}: Readonly<{
  userName: string | undefined
  userEmail: string | null | undefined
  onLogout: () => void | Promise<void>
  /** The element that opens the menu (the sidebar's footer card); it receives the menu's props and ref. */
  trigger?: React.ReactElement
  /** What `trigger` shows. */
  children?: React.ReactNode
}>) {
  const { theme, setTheme, resolvedTheme } = useTheme()
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

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          trigger ?? (
            <Button
              variant="ghost"
              size="icon"
              className="size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
              aria-label="Account menu"
            />
          )
        }
      >
        {trigger ? children : <CircleUserRound className="size-5" />}
      </DropdownMenuTrigger>
      {/* From the sidebar's footer the menu opens upward (it flips from below), as wide as the card. */}
      <DropdownMenuContent align="start" className="w-(--anchor-width) min-w-56">
        <DropdownMenuLabel>
          <p className="truncate text-sm font-medium text-foreground">{userName ?? "Signed in"}</p>
          {userEmail ? <p className="truncate text-xs font-normal text-muted-foreground">{userEmail}</p> : null}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem render={<Link href="/account/change-password" />}>
          <KeyRound />
          Change password
        </DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <ThemeIcon mounted={mounted} theme={theme} resolvedTheme={resolvedTheme} />
            <span>Theme</span>
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuRadioGroup value={mounted ? theme : undefined} onValueChange={(val) => setTheme(val)}>
              <DropdownMenuRadioItem value="light">
                <Sun className="size-4" />
                <span>Light</span>
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="dark">
                <Moon className="size-4" />
                <span>Dark</span>
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="system">
                <Monitor className="size-4" />
                <span>System</span>
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        {/* REL-2 S1-T5: the console's own version, so a person can say
            which build they are looking at. Disabled — it is a fact, not
            an action. */}
        <DropdownMenuItem disabled>
          Version {CONSOLE_VERSION}
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onClick={() => void onLogout()}>
          <LogOut />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
