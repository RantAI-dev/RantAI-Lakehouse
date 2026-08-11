import { redirect } from "next/navigation"

/** Legacy route compatibility redirect. */
export default function LegacyRedirect() {
  redirect("/admin/tenants")
}
