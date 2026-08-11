"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"
import { tabsListVariants } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

const TABS = [
  { href: "/query-studio", label: "Studio" },
  { href: "/query-studio/saved", label: "Saved Queries" },
  { href: "/query-studio/collaboration", label: "Collaboration" },
] as const

/** Internal navigation between the Query Studio surfaces. */
export function QueryStudioTabs() {
  const pathname = usePathname()
  return (
    <nav
      aria-label="Query Studio sections"
      className={cn(tabsListVariants({ variant: "default" }), "h-8")}
    >
      {TABS.map((tab) => {
        const active = pathname === tab.href
        return (
          <Link
            key={tab.href}
            href={tab.href}
            aria-current={active ? "page" : undefined}
            className={cn(
              "inline-flex h-[calc(100%-1px)] items-center justify-center rounded-md border border-transparent px-2.5 py-0.5 text-sm font-medium whitespace-nowrap transition-all",
              active
                ? "bg-background text-foreground shadow-sm dark:border-input dark:bg-input/30"
                : "text-foreground/60 hover:text-foreground dark:text-muted-foreground dark:hover:text-foreground"
            )}
          >
            {tab.label}
          </Link>
        )
      })}
    </nav>
  )
}
