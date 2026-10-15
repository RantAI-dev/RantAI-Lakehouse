"use client"

import * as React from "react"
import type { AlertItem } from "@/services/contracts/overview"

type DeliveryFields = Pick<AlertItem, "delivered" | "deliveryReason">

/**
 * How an alert's delivery went, in words (`SRC-17`, load-failure alerts
 * checklist row 5). `unknown` is an alert saved before the outcome was kept:
 * it says nothing rather than "Delivered". The reason is the API's own fixed
 * sentence; nothing here adds to it.
 */
export function deliverySummary(alert: DeliveryFields): {
  state: "delivered" | "not-delivered" | "unknown"
  text: string
} {
  if (alert.delivered === true) return { state: "delivered", text: "Delivered" }
  if (alert.delivered === false) {
    return {
      state: "not-delivered",
      text: alert.deliveryReason
        ? `Not delivered: ${alert.deliveryReason}`
        : "Not delivered",
    }
  }
  return { state: "unknown", text: "" }
}

/** The delivery outcome of an alert, or a dash when it is not known. */
export function AlertDelivery({ alert }: { readonly alert: DeliveryFields }) {
  const summary = deliverySummary(alert)
  if (summary.state === "unknown") {
    return <span className="text-xs text-muted-foreground">—</span>
  }
  return (
    <span
      className={
        summary.state === "not-delivered"
          ? "text-xs text-destructive"
          : "text-xs text-muted-foreground"
      }
    >
      {summary.text}
    </span>
  )
}
