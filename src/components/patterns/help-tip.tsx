"use client"

import { Popover as PopoverPrimitive } from "@base-ui/react/popover"
import { CircleHelp } from "lucide-react"

import { cn } from "@/lib/utils"

/**
 * A quiet ? right after a section heading, holding the detail the one-line
 * helper under it leaves out (`UX-7`, owner QA 2026-11-20: help text in the
 * app is one short line, details go in a tip; `DESIGN.md` §2).
 *
 * A tooltip proper opens on hover and focus only, which a phone has neither
 * of, so this is a popover that opens on hover and on a press (a tap, or
 * Enter and Space on the focused button). Unlike `HelpHint`, whose accessible
 * name is the detail itself, the name here is "More about <heading>": the
 * button sits beside a heading, so that is what a screen reader should say,
 * and the detail is read when it opens. Use `HelpHint` for a detail that
 * follows a line of text instead of a heading.
 */
export function HelpTip({ heading, children, className }: {
  readonly heading: string
  readonly children: React.ReactNode
  readonly className?: string
}) {
  return (
    <PopoverPrimitive.Root>
      <PopoverPrimitive.Trigger
        openOnHover
        delay={150}
        aria-label={`More about ${heading}`}
        className={cn(
          "inline-flex size-5 shrink-0 cursor-help items-center justify-center rounded-full text-muted-foreground/80 outline-none transition-colors duration-150 ease-out hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring active:bg-muted/70 data-[popup-open]:bg-muted data-[popup-open]:text-foreground motion-reduce:transition-none",
          className
        )}
      >
        <CircleHelp className="size-3.5" aria-hidden />
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Positioner side="top" sideOffset={6} collisionPadding={12} className="isolate z-[60]">
          <PopoverPrimitive.Popup
            className="max-w-70 origin-(--transform-origin) rounded-md bg-foreground px-3 py-2 text-xs leading-relaxed text-background shadow-md outline-none transition-[opacity,transform] duration-150 ease-out data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0 motion-reduce:transition-none"
          >
            {children}
          </PopoverPrimitive.Popup>
        </PopoverPrimitive.Positioner>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  )
}
