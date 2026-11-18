"use client"

import { UsagePage } from "@/features/usage/usage-page"

/**
 * Thin App Router page for Usage (`OPS-8` part B): the runs of a period
 * summed per day, person or engine, a CSV of the same sums, and the rates
 * behind the estimate in money.
 */
export default function Page() {
  return <UsagePage />
}
