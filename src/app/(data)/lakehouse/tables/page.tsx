import { redirect } from "next/navigation"

/**
 * The Iceberg table list is gone (`WEB-3` S2-D2): a table is reached from
 * its asset page in the Catalog, and its maintenance runs are a section of
 * System status. Old links and bookmarks land on the Catalog. A table's own
 * page (`/lakehouse/tables/{namespace}/{table}`) is unchanged.
 */
export default function LegacyRedirect() {
  redirect("/catalog")
}
