import { redirect } from "next/navigation"

/**
 * The Iceberg table list moved into Table Maintenance: the per-table
 * maintenance policy was the only thing this page did that no other page
 * did, so it now sits next to the maintenance runs it drives. Old links
 * and bookmarks land there instead of 404ing.
 */
export default function LegacyRedirect() {
  redirect("/governance/maintenance")
}
