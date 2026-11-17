import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` S2-D2): Table Maintenance is a section of
 * System status now.
 */
export default function LegacyRedirect() {
  redirect("/health#maintenance")
}
