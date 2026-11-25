"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"
import { cn } from "@/lib/utils"

const TABS = [
  { href: "/query-studio", label: "Studio" },
  { href: "/query-studio/saved", label: "Saved" },
  { href: "/query-studio/history", label: "History" },
] as const

/**
 * The Query Studio sections: Studio · Saved · History, one segmented control
 * on the right of each section's title row (`UX-9`, layout A). They are
 * pages, so each segment is a link; the open one is `aria-current`.
 *
 * The Studio page used to repeat Saved and History as cards beside the
 * editor; those are gone, and this control is the one way between them.
 */
export function QueryStudioTabs({ className }: { readonly className?: string }) {
  const pathname = usePathname()
  return (
    <nav
      aria-label="Query Studio sections"
      className={cn(
        "inline-flex h-8 shrink-0 items-center rounded-lg bg-muted p-0.5 dark:bg-input/30",
        className
      )}
    >
      {TABS.map((tab) => {
        const active = pathname === tab.href
        return (
          <Link
            key={tab.href}
            href={tab.href}
            aria-current={active ? "page" : undefined}
            className={cn(
              "inline-flex h-7 items-center rounded-md px-2.5 text-[13px] font-medium whitespace-nowrap outline-none transition-[color,background-color,box-shadow] duration-150 ease-ui focus-visible:ring-3 focus-visible:ring-ring/50",
              active
                ? "bg-background text-foreground shadow-xs dark:bg-input/60"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            {tab.label}
          </Link>
        )
      })}
    </nav>
  )
}
