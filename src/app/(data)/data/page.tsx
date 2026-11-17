import { redirect } from "next/navigation"
import { catalogRedirectHref } from "@/lib/catalog-redirect"

/**
 * Legacy route compatibility redirect (`WEB-3` S2-D1): Data Explorer and
 * Catalog were one asset list twice, and the list is `/catalog` now. The
 * query parameters that still mean something there (search term, filters,
 * sort) travel with the redirect; the rest are dropped.
 */
export default async function LegacyRedirect({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  redirect(catalogRedirectHref(await searchParams))
}
