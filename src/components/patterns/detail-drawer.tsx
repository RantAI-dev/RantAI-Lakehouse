"use client"

import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { cn } from "@/lib/utils"

/**
 * Right-side drawer for quick entity inspection from list rows.
 * Durable entities should also have a dedicated detail route; the drawer is
 * the fast path, not a replacement.
 */
export function DetailDrawer({
  open,
  onOpenChange,
  title,
  description,
  children,
  wide = false,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  children: React.ReactNode
  wide?: boolean
}) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className={cn(
          "gap-0 overflow-y-auto p-0 sm:max-w-md",
          wide && "sm:max-w-xl"
        )}
      >
        <SheetHeader className="sticky top-0 z-10 border-b border-border bg-background">
          <SheetTitle>{title}</SheetTitle>
          {description ? (
            <SheetDescription>{description}</SheetDescription>
          ) : null}
        </SheetHeader>
        <div className="flex flex-col gap-5 px-6 py-5">{children}</div>
      </SheetContent>
    </Sheet>
  )
}
