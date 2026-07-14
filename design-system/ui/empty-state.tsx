import * as React from "react"
import { cn } from "../lib/utils"

function EmptyState({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-state"
      className={cn(
        "flex flex-col items-center justify-center gap-4 py-20 text-center",
        className
      )}
      {...props}
    />
  )
}

function EmptyStateIcon({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-state-icon"
      className={cn(
        "text-muted-foreground/40 [&>svg]:size-12 [&>svg]:stroke-[1.5]",
        className
      )}
      {...props}
    />
  )
}

function EmptyStateTitle({
  className,
  ...props
}: React.ComponentProps<"p">) {
  return (
    <p
      data-slot="empty-state-title"
      className={cn("text-sm font-medium text-foreground", className)}
      {...props}
    />
  )
}

function EmptyStateDescription({
  className,
  ...props
}: React.ComponentProps<"p">) {
  return (
    <p
      data-slot="empty-state-description"
      className={cn("font-mono text-xs text-muted-foreground", className)}
      {...props}
    />
  )
}

function EmptyStateAction({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-state-action"
      className={cn("mt-2", className)}
      {...props}
    />
  )
}

export {
  EmptyState,
  EmptyStateIcon,
  EmptyStateTitle,
  EmptyStateDescription,
  EmptyStateAction,
}
