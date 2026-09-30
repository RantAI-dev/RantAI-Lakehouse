import { cn } from "@/lib/utils"

/**
 * A light band that sweeps across its parent on hover. The parent must be
 * `relative overflow-hidden` and name a Tailwind group; the caller passes
 * the matching `group-hover/<name>:translate-x-[400%]` class, because
 * Tailwind only generates classes it sees as literals. Hidden under
 * `prefers-reduced-motion`.
 */
export function Sheen({ className }: { className: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        "pointer-events-none absolute inset-y-0 -left-1/2 w-1/2 -translate-x-full skew-x-[-20deg] bg-gradient-to-r from-transparent via-white/25 to-transparent transition-transform duration-700 ease-out motion-reduce:hidden dark:via-white/10",
        className
      )}
    />
  )
}
