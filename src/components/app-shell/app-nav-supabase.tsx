"use client"

import * as React from "react"
import Image from "next/image"
import Link from "next/link"
import { usePathname, useRouter } from "next/navigation"
import { CircleUserRound } from "lucide-react"
import { useSidebar } from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"
import { visibleNavGroups, activeNavHref, type NavGroup } from "./nav-config"

/**
 * Navigasi dua-panel gaya SUPABASE: rail ikon tipis (section) + panel sekunder
 * yang PERSISTEN menampilkan isi section yang sedang dilihat. Panel mengikuti
 * halaman aktif; klik ikon rail = pindah section (+ buka halaman pertamanya).
 * Panel disembunyikan saat sidebar diciutkan (dan untuk section 1-item).
 */
export function AppNavSupabase() {
  const pathname = usePathname()
  const router = useRouter()
  const activeHref = activeNavHref(pathname)
  const { state } = useSidebar()
  const groups = visibleNavGroups()

  const activeGroup = groups.find((g) => g.items.some((it) => it.href === activeHref))
  const [viewed, setViewed] = React.useState<string | undefined>(activeGroup?.label)
  React.useEffect(() => { if (activeGroup) setViewed(activeGroup.label) }, [activeGroup])

  const viewedGroup = groups.find((g) => g.label === viewed)
  const showPanel = state === "expanded" && !!viewedGroup && viewedGroup.items.length > 1

  const onRail = (g: NavGroup) => {
    setViewed(g.label)
    router.push(g.items[0].href)
  }

  return (
    <aside className="flex h-svh shrink-0 border-r border-sidebar-border bg-sidebar">
      {/* Rail ikon */}
      <div className="flex w-14 shrink-0 flex-col items-center gap-1 py-3">
        <Link href="/" className="relative mb-2 size-8 overflow-hidden rounded-md" aria-label="Beranda">
          <Image src="/rantai.png" alt="Rantai Lake" fill sizes="32px" className="object-cover" priority />
        </Link>
        <nav className="flex flex-1 flex-col items-center gap-1">
          {groups.map((g) => {
            const Icon = g.icon ?? g.items[0].icon
            const isActive = g.label === activeGroup?.label
            const isViewed = g.label === viewed
            return (
              <button
                key={g.label}
                type="button"
                title={g.label}
                onClick={() => onRail(g)}
                aria-current={isActive ? "page" : undefined}
                className={cn(
                  "grid size-10 place-items-center rounded-lg text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground",
                  isActive && "bg-sidebar-accent text-sidebar-primary ring-1 ring-sidebar-border",
                  !isActive && isViewed && "text-sidebar-foreground",
                )}
              >
                <Icon className="size-5" />
              </button>
            )
          })}
        </nav>
        <button
          type="button"
          title="Admin User"
          className="grid size-10 place-items-center rounded-lg text-sidebar-foreground/70 hover:bg-sidebar-accent"
        >
          <span className="relative grid size-8 place-items-center rounded-full bg-primary/12 text-primary ring-1 ring-primary/20">
            <CircleUserRound className="size-5" />
            <span className="absolute bottom-0 right-0 size-2 rounded-full border-2 border-sidebar bg-emerald-500" />
          </span>
        </button>
      </div>

      {/* Panel sekunder */}
      {showPanel && viewedGroup ? (
        <div className="flex w-56 shrink-0 flex-col border-l border-sidebar-border px-2 py-3">
          <div className="px-2 pb-2">
            <p className="text-sm font-semibold text-sidebar-foreground">{viewedGroup.label}</p>
            <p className="text-[11px] text-sidebar-foreground/60">Rantai Lake</p>
          </div>
          <nav className="flex flex-col gap-0.5">
            {viewedGroup.items.map((it) => {
              const Icon = it.icon
              const active = it.href === activeHref
              return (
                <Link
                  key={it.href}
                  href={it.href}
                  className={cn(
                    "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                    active && "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border",
                  )}
                >
                  <Icon className="size-4 shrink-0" />
                  <span className="truncate">{it.title}</span>
                </Link>
              )
            })}
          </nav>
        </div>
      ) : null}
    </aside>
  )
}
