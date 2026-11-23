"use client"

import * as React from "react"
import Link from "next/link"
import { Copyable } from "@/components/copyable"
import { CodeBlock } from "@/components/patterns/code-block"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useService } from "@/hooks/use-service"
import { goldService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import type { GoldReadBack } from "@/services/contracts/gold"
import {
  buildIcebergSnippets,
  buildRestApiSnippet,
  EXPORTED_AT_NOTE,
  GOVERNED_API_NOTE,
  ICEBERG_REST_PATH,
  martFromAssetId,
  QUERY_RUN_PATH,
  QUERY_TOKEN_NOTE,
  SERVICE_IDENTITIES_HREF,
} from "./asset-connect-snippets"

/** One snippet and its copy control; used by each Iceberg client sub-tab. */
function SnippetPanel({ value, snippet }: { value: string; snippet: string }) {
  return (
    <TabsContent value={value} className="mt-0">
      <div className="mb-1 flex justify-end">
        <Copyable value={snippet}>
          <span className="text-xs">Copy snippet</span>
        </Copyable>
      </div>
      <CodeBlock>{snippet}</CodeBlock>
    </TabsContent>
  )
}

export type AssetConnectDialogProps = {
  asset: AssetDetail
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * The compact "Connect" dialog on an asset's detail page.
 *
 * The REST tab is offered for every zone. The Iceberg tab is offered only
 * when the catalog's own read-back says the Gold copy exists — `enabled` on
 * the publication is not enough, because a mart can be switched on without
 * a copy having been exported, or after its copy was removed. A failed or
 * in-flight read never yields a tab: that would be a broken snippet.
 */
export function AssetConnectDialog({ asset, open, onOpenChange }: AssetConnectDialogProps) {
  const isGold = asset.layer === "gold"
  const mart = martFromAssetId(asset.id)
  const iceberg = useService<GoldReadBack | null>(async (signal) => {
    if (!open || !isGold) return null
    return await goldService.getLastExport(mart, signal)
  }, [open, isGold, mart])

  const published =
    iceberg.status === "success" && iceberg.data?.published === true ? iceberg.data : null

  // Both builders are pure. The REST id is the registry's fully-qualified
  // asset id; the Iceberg names come from the read-back, not from a guess
  // built out of the layer and the mart name.
  const restSnippet = buildRestApiSnippet({ qualifiedTable: asset.id })
  const icebergSnippets = published
    ? buildIcebergSnippets({ namespace: published.namespace, table: published.table })
    : null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Connect to {asset.name}</DialogTitle>
          <DialogDescription>
            Copy a ready-to-run snippet for a governed tool. No raw database
            connection is offered: every read goes through an audited API door.
          </DialogDescription>
        </DialogHeader>
        <Tabs defaultValue="rest" className="gap-3">
          <TabsList>
            <TabsTrigger value="rest">Governed REST API</TabsTrigger>
            {published ? (
              <TabsTrigger value="iceberg">Iceberg REST Catalog</TabsTrigger>
            ) : null}
          </TabsList>

          <TabsContent value="rest" className="mt-0 grid gap-3">
            <p className="text-sm text-muted-foreground">{GOVERNED_API_NOTE}</p>
            <div>
              <div className="mb-1 flex items-center justify-between gap-2">
                <span className="text-sm font-medium">POST {QUERY_RUN_PATH}</span>
                <Copyable value={restSnippet}>
                  <span className="text-xs">Copy snippet</span>
                </Copyable>
              </div>
              <CodeBlock>{restSnippet}</CodeBlock>
            </div>
            <p className="text-xs text-muted-foreground">{QUERY_TOKEN_NOTE}</p>
            <p className="text-xs">
              <Link href={SERVICE_IDENTITIES_HREF} className="text-primary hover:underline">
                Open Service Identities
              </Link>{" "}
              (Settings › Service Identities) to create a service token for the
              governed API.
            </p>
          </TabsContent>

          {published && icebergSnippets ? (
            <TabsContent value="iceberg" className="mt-0 grid gap-3">
              <p className="text-sm text-muted-foreground">
                Outside tools read the published Gold table through the Iceberg
                REST catalog door at{" "}
                <code className="font-mono">{ICEBERG_REST_PATH}</code>. Reads are
                counted per service identity.
              </p>
              <p className="text-xs text-muted-foreground">
                Use a token with the iceberg:read scope from{" "}
                <Link href={SERVICE_IDENTITIES_HREF} className="text-primary hover:underline">
                  Service Identities
                </Link>
                .
              </p>
              <Tabs defaultValue="duckdb" className="gap-2">
                <TabsList>
                  <TabsTrigger value="duckdb">DuckDB</TabsTrigger>
                  <TabsTrigger value="pyiceberg">PyIceberg</TabsTrigger>
                  <TabsTrigger value="spark">Spark</TabsTrigger>
                </TabsList>
                <SnippetPanel value="duckdb" snippet={icebergSnippets.duckdb} />
                <SnippetPanel value="pyiceberg" snippet={icebergSnippets.pyiceberg} />
                <SnippetPanel value="spark" snippet={icebergSnippets.spark} />
              </Tabs>
              <p className="text-xs text-muted-foreground">{EXPORTED_AT_NOTE}</p>
            </TabsContent>
          ) : null}
        </Tabs>
      </DialogContent>
    </Dialog>
  )
}
