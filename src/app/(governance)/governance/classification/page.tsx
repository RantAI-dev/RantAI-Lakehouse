import { redirect } from "next/navigation"

/**
 * Legacy route compatibility redirect (`WEB-3` S3-D3): a column's
 * sensitivity and masks are set on the column itself, in a table's Schema
 * tab, so the Classification & Masking page is gone. Its list showed one
 * invented "internal" row per dataset (section 6.1).
 */
export default function LegacyRedirect() {
  redirect("/catalog")
}
