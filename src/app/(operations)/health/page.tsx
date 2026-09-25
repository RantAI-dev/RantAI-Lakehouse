"use client"

import { OverviewPage } from "@/features/overview/overview-page"

/**
 * Thin App Router page for OverviewPage. The platform dashboard moved here
 * from `/` when Home became the AI-first landing page.
 */
export default function Page() {
  return <OverviewPage />
}
