"use client"

import * as React from "react"
import { useService } from "@/hooks/use-service"
import { mapService } from "@/services"
import { MAP_CATALOGUE, uploadedEntry, type MapEntry } from "./echarts-maps"

/**
 * The maps a chart can be drawn on: the bundled ones and the administrator's
 * uploaded ones (`BI-16` part C2b), the latter by the name they were given. A
 * failed or missing list leaves the bundled maps (the uploaded route is not on
 * every server yet); a chart already on an uploaded map still draws, because
 * its id is enough (`mapEntry`).
 */
export function useMapCatalogue(): readonly MapEntry[] {
  const uploaded = useService((signal) => mapService.list(signal), [])
  return React.useMemo(
    () => (uploaded.status === "success" ? [...MAP_CATALOGUE, ...uploaded.data.map((m) => uploadedEntry(m.id, m.name))] : MAP_CATALOGUE),
    [uploaded],
  )
}
