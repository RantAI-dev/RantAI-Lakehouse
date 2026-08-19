"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"
import { useSidebar } from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"
import { activeNavHref, subNavItems } from "./nav-config"

/**
 * Bottom nav — sub-halaman dari section yang aktif, tampil sebagai bar di BAWAH
 * konten (dipisah garis). Hanya saat sidebar MELEBAR; ketika diciutkan, sub-menu
 * diakses lewat FLYOUT klik di sidebar (lihat AppSidebar).
 */
export function AppBottomNav() {
  const pathname = usePathname()
  const { state } = useSidebar()
  const items = subNavItems(pathname)
  const activeHref = activeNavHref(pathname)

  if (state !== "expanded" || items.length === 0) return null

  return (
    <nav
      aria-label="Sub-navigasi section"
      className="sticky bottom-0 z-30 flex items-center gap-1 overflow-x-auto border-t border-border bg-background/95 px-3 py-1.5 backdrop-blur-md sm:px-4"
    >
      {items.map((it) => {
        const active = it.href === activeHref
        const Icon = it.icon
        return (
          <Link
            key={it.href}
            href={it.href}
            className={cn(
              "flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm transition-colors",
              active ? "bg-primary/10 font-medium text-primary" : "text-muted-foreground hover:bg-muted hover:text-foreground",
            )}
          >
            <Icon className="size-4 shrink-0" />
            {it.title}
          </Link>
        )
      })}
    </nav>
  )
}
