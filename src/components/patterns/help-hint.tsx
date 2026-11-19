"use client"

import { Popover as PopoverPrimitive } from "@base-ui/react/popover"
import { CircleHelp } from "lucide-react"

import { cn } from "@/lib/utils"

/**
 * A quiet ⓘ after a one-line helper, holding the rest of what the line says.
 *
 * Owner QA 2026-11-16 (the alert and subscriptions dialogs): help and status
 * text in a form is one short line, and its detail goes in a tooltip
 * (`DESIGN.md` §2). A tooltip proper opens on hover and focus only, which a
 * phone has neither of, so this is a popover that opens on hover and on a
 * press (a tap, or Enter and Space on the focused button).
 *
 * The button's accessible name is the detail itself, so a screen reader hears
 * the whole sentence without opening anything.
 */
export function HelpHint({ children, className }: { readonly children: string; readonly className?: string }) {
  return (
    <PopoverPrimitive.Root>
      <PopoverPrimitive.Trigger
        openOnHover
        delay={150}
        aria-label={children}
        className={cn(
          "inline-flex size-4 shrink-0 cursor-help items-center justify-center rounded-full align-[-3px] text-muted-foreground/80 outline-none transition-colors duration-150 ease-out hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[popup-open]:text-foreground motion-reduce:transition-none",
          className
        )}
      >
        <CircleHelp className="size-3.5" aria-hidden />
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Positioner side="top" sideOffset={6} collisionPadding={12} className="isolate z-[60]">
          <PopoverPrimitive.Popup
            className="max-w-72 origin-(--transform-origin) rounded-md bg-foreground px-3 py-2 text-xs leading-relaxed text-background shadow-md outline-none transition-[opacity,transform] duration-150 ease-out data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0 motion-reduce:transition-none"
          >
            {children}
          </PopoverPrimitive.Popup>
        </PopoverPrimitive.Positioner>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  )
}
