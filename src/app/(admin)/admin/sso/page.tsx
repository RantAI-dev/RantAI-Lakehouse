import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` D6, S3-D6): sign-in
 * configuration is the "Sign-in" section of `/admin/settings`.
 */
export default function LegacyRedirect() {
  redirect("/admin/settings?section=sign-in")
}
