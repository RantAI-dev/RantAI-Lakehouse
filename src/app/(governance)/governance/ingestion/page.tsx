import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` S2-D2): Ingestion (CDC) is a section of
 * System status now.
 */
export default function LegacyRedirect() {
  redirect("/health#change-capture")
}
