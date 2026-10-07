"use client"

import { useParams } from "next/navigation"
import { ConnectorEditPage } from "@/features/connectors/connector-edit-page"

/** Thin App Router page for ConnectorEditPage. */
export default function Page() {
  const params = useParams<{ id: string }>()
  return <ConnectorEditPage connectorId={params.id} />
}
