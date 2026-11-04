"use client"

import { SystemStatusPage } from "@/features/overview/system-status-page"

/**
 * Thin App Router page for System status (`WEB-3` S2-D2). The address
 * stays `/health`; Services, Capacity, Ingestion (CDC) and Table
 * Maintenance redirect to its sections.
 */
export default function Page() {
  return <SystemStatusPage />
}
