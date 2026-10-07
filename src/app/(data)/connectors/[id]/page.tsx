"use client"

import { useParams } from "next/navigation"
import { ConnectorDetailPage } from "@/features/connectors/connector-detail-page"

/** Thin App Router page for ConnectorDetailPage. */
export default function Page() {
  const params = useParams<{ id: string }>()
  return <ConnectorDetailPage connectorId={params.id} />
}
