import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` D6): sign-in configuration
 * moved into `/admin/settings` as a "Sign-in" section.
 */
export default function LegacyRedirect() {
  redirect("/admin/settings")
}
