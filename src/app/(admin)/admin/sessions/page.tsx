import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` S3-D7): sessions are the
 * Sessions tab of Users now.
 */
export default function LegacyRedirect() {
  redirect("/admin/users?tab=sessions")
}
