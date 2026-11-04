"use client"

import Link from "next/link"
import { tableKeyAssetHref } from "@/lib/table-key-href"

/**
 * A table key (`silver.orders`) as a link to its asset page when the key is
 * an asset id, and as plain text when it is not (a Bronze key: `WEB-3`
 * S3-D5, no fabricated link). `tab` opens one of the asset page's tabs.
 */
export function TableKeyLink({ tableKey, tab }: { tableKey: string; tab?: string }) {
  const href = tableKeyAssetHref(tableKey, tab)
  if (!href) return <span className="font-mono text-xs">{tableKey}</span>
  return (
    <Link href={href} className="font-mono text-xs underline-offset-2 hover:underline">
      {tableKey}
    </Link>
  )
}
