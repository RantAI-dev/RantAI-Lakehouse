"use client"

import { AlertTriangle, Inbox, Lock, RotateCcw } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import type { ServiceError } from "@/services/errors"

function StateShell({
  icon,
  title,
  description,
  action,
  className,
}: {
  icon: React.ReactNode
  title: string
  description?: string
  action?: React.ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border bg-muted/20 px-6 py-12 text-center",
        className
      )}
      role="status"
    >
      <div className="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
        {icon}
      </div>
      <p className="text-sm font-semibold text-foreground">{title}</p>
      {description ? (
        <p className="max-w-md text-sm text-muted-foreground">{description}</p>
      ) : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  )
}

/** Empty result state with optional call to action. */
export function EmptyState({
  title,
  description,
  action,
  className,
}: {
  title: string
  description?: string
  action?: React.ReactNode
  className?: string
}) {
  return (
    <StateShell
      icon={<Inbox className="size-5" aria-hidden />}
      title={title}
      description={description}
      action={action}
      className={className}
    />
  )
}

/** Error state with retry. Pass the normalized ServiceError from useService. */
export function ErrorState({
  error,
  onRetry,
  className,
}: {
  error: ServiceError
  onRetry?: () => void
  className?: string
}) {
  if (error.code === "permission_denied") {
    return <PermissionState className={className} />
  }
  return (
    <StateShell
      icon={<AlertTriangle className="size-5" aria-hidden />}
      title="Something went wrong"
      description={error.message}
      action={
        onRetry ? (
          <Button variant="outline" size="sm" onClick={onRetry}>
            <RotateCcw className="size-3.5" aria-hidden />
            Retry
          </Button>
        ) : undefined
      }
      className={className}
    />
  )
}

/** Shown when the current identity is not allowed to view a resource. */
export function PermissionState({ className }: { className?: string }) {
  return (
    <StateShell
      icon={<Lock className="size-5" aria-hidden />}
      title="You don't have access"
      description="Your role does not permit viewing this resource. Request access from a workspace administrator."
      className={className}
    />
  )
}

/** Table-shaped loading skeleton used while a list request is in flight. */
export function LoadingSkeleton({
  rows = 6,
  className,
}: {
  rows?: number
  className?: string
}) {
  return (
    <div
      className={cn("flex flex-col gap-2", className)}
      role="status"
      aria-label="Loading"
    >
      <Skeleton className="h-9 w-full max-w-sm" />
      {Array.from({ length: rows }).map((_, i) => (
        <Skeleton key={i} className="h-12 w-full" />
      ))}
    </div>
  )
}

/** Card-grid loading skeleton for KPI strips. */
export function MetricSkeleton({
  cards = 4,
  className,
}: {
  cards?: number
  className?: string
}) {
  return (
    <div
      className={cn("grid gap-3 sm:grid-cols-2 lg:grid-cols-4", className)}
      role="status"
      aria-label="Loading metrics"
    >
      {Array.from({ length: cards }).map((_, i) => (
        <Skeleton key={i} className="h-24 w-full" />
      ))}
    </div>
  )
}
