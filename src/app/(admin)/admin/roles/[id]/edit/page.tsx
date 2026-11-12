"use client"

import { Suspense } from "react"
import { LoadingSkeleton } from "@/components/patterns/page-states"
import { RoleEditPage } from "@/features/admin/role-form-page"

/**
 * `WEB-7` D2: the edit-role form's own page, loaded by id.
 * `GOV-8` D3: the page reads `?tables=unsaved` through `useSearchParams`,
 * which needs a Suspense boundary in the App Router.
 */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton />}>
      <RoleEditPage />
    </Suspense>
  )
}
