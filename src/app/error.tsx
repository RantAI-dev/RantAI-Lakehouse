"use client"

import { useEffect } from "react"
import { Button } from "@/components/ui/button"

/**
 * Route-level error boundary with on-brand recovery actions.
 */
export default function AppError({
  error,
  reset,
}: {
  error: Error & { digest?: string }
  reset: () => void
}) {
  useEffect(() => {
    console.error(error)
  }, [error])

  return (
    <div className="flex min-h-[50vh] flex-col items-center justify-center gap-3 px-4 text-center">
      <h2 className="text-lg font-semibold">Something went wrong</h2>
      <p className="max-w-md text-sm text-muted-foreground">
        {error.message || "An unexpected error occurred while rendering this page."}
      </p>
      <Button size="sm" onClick={reset}>
        Try again
      </Button>
    </div>
  )
}
