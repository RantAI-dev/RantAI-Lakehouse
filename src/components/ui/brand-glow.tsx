"use client"

import * as React from "react"
import { motion, useReducedMotion } from "motion/react"

import { cn } from "@/lib/utils"

/**
 * The console's "light through water" treatment, shared by Home and the
 * Copilot page. It follows `/login`'s WebGL hero in the brand blues
 * (`--brand-1`, `--brand-canvas-*`), done with CSS blur and `motion` so it
 * stays cheap on pages people keep open. All movement stops under
 * `prefers-reduced-motion`.
 */

/**
 * Two slow-drifting brand glows over a faint grid. Place it first inside a
 * `relative isolate overflow-hidden` container; it sits behind the content.
 */
export function BrandBackdrop({ className }: { className?: string }) {
  const reduce = useReducedMotion() ?? false
  const drift = (x: number[], y: number[], duration: number) =>
    reduce
      ? {}
      : {
          animate: { x, y },
          transition: {
            duration,
            repeat: Infinity,
            repeatType: "mirror" as const,
            ease: "easeInOut" as const,
          },
        }
  return (
    <div
      aria-hidden
      className={cn("pointer-events-none absolute inset-0 -z-10", className)}
    >
      <motion.div
        {...drift([0, 60, -20], [0, 20, -10], 18)}
        className="absolute -top-24 left-[8%] size-72 rounded-full bg-[var(--brand-canvas-light)] opacity-25 blur-[90px] dark:opacity-20"
      />
      <motion.div
        {...drift([0, -50, 30], [0, -15, 25], 22)}
        className="absolute -right-10 top-10 size-80 rounded-full bg-[var(--brand-canvas-dark)] opacity-30 blur-[100px] dark:opacity-40"
      />
      <div
        className="absolute inset-0 opacity-[0.35] [mask-image:radial-gradient(ellipse_at_center,black_20%,transparent_70%)] dark:opacity-[0.18]"
        style={{
          backgroundImage:
            "linear-gradient(to right, var(--border) 1px, transparent 1px), linear-gradient(to bottom, var(--border) 1px, transparent 1px)",
          backgroundSize: "32px 32px",
        }}
      />
    </div>
  )
}

/**
 * A 1px frame around an input whose brand-blue ring spins while anything
 * inside has focus. Tracks focus itself (`onFocus`/`onBlur` bubble in
 * React), so callers only wrap their surface.
 */
export function GlowFrame({
  children,
  className,
  innerClassName,
}: {
  children: React.ReactNode
  className?: string
  innerClassName?: string
}) {
  const reduce = useReducedMotion() ?? false
  const [focused, setFocused] = React.useState(false)
  return (
    <div
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        // Moving focus between the textarea and the toolbar stays "focused".
        if (!e.currentTarget.contains(e.relatedTarget as Node | null))
          setFocused(false)
      }}
      className={cn("relative rounded-2xl p-px", className)}
    >
      <div
        aria-hidden
        className={cn(
          "absolute inset-0 overflow-hidden rounded-[inherit] bg-border transition-opacity duration-300",
          focused ? "opacity-100" : "opacity-60",
        )}
      >
        <motion.div
          className={cn(
            "absolute top-1/2 left-1/2 aspect-square w-[200%] -translate-x-1/2 -translate-y-1/2 transition-opacity duration-500",
            focused ? "opacity-100" : "opacity-0",
          )}
          style={{
            background:
              "conic-gradient(from 0deg, transparent 0deg, var(--brand-1) 60deg, var(--brand-canvas-dark) 120deg, transparent 180deg, transparent 360deg)",
          }}
          animate={reduce ? undefined : { rotate: 360 }}
          transition={{ duration: 6, repeat: Infinity, ease: "linear" }}
        />
      </div>
      <div
        className={cn(
          "relative rounded-[15px] bg-background/90 shadow-[0_20px_60px_-30px_var(--brand-canvas-dark)] backdrop-blur-xl",
          innerClassName,
        )}
      >
        {children}
      </div>
    </div>
  )
}
