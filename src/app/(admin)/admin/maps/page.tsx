import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` S3-D6): Maps is a section of
 * Settings now.
 */
export default function LegacyRedirect() {
  redirect("/admin/settings?section=maps")
}
