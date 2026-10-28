/**
 * Uploaded map boundaries (`BI-16` part C2b, plan D5): a GeoJSON file an
 * administrator uploads, names and chooses the name property of, then used for
 * region, pin and grid maps like a built-in map. **None of these routes is on
 * the server yet** (T5 and T6 of the bi-16c plan); the plan names the routes
 * and their permissions, not the JSON, so the shapes below are the console's
 * choice and the contract for T6.
 *
 * - `GET /api/maps` (`RequiresAuth`): `{ maps: UploadedMap[] }`.
 * - `POST /api/maps` (`settings:write`, multipart, up to 50 MB): fields `file`,
 *   `name`, `nameProperty`; answers the `UploadedMap`. A refused file answers
 *   400 or 413 with `{ error }`, a plain sentence.
 * - `PATCH /api/maps/{id}` (`settings:write`): `{ name }`; answers the map.
 * - `DELETE /api/maps/{id}` (`settings:write`): `{ ok: true }`, or 409 with
 *   `{ error, charts: MapUser[] }` when a chart uses the map.
 * - `GET /api/maps/{id}/geojson` (`RequiresAuth`): the simplified file, with
 *   the chosen name property copied to `properties.name` on every feature, so
 *   the console reads an uploaded map the way it reads a built-in one.
 */
export type UploadedMap = {
  /** Server-made, `m_` and 12 hex digits. */
  id: string
  name: string
  /** The property of the uploaded file that held each region's name. */
  nameProperty: string
  regionCount: number
  createdAt: string
  createdBy?: string
}

/** A chart that uses a map, named when a delete is refused. */
export type MapUser = { chartId: string; chartTitle: string; dashboardId: string; dashboardTitle: string }

export type UploadMapInput = { file: File; name: string; nameProperty: string }

export type DeleteMapResult = { ok: true } | { ok: false; message: string; charts: MapUser[] }

export interface MapService {
  list(signal?: AbortSignal): Promise<UploadedMap[]>
  upload(input: UploadMapInput, signal?: AbortSignal): Promise<UploadedMap>
  rename(id: string, name: string, signal?: AbortSignal): Promise<UploadedMap>
  /** A map in use is not deleted: the result lists the charts that use it. */
  remove(id: string, signal?: AbortSignal): Promise<DeleteMapResult>
}
