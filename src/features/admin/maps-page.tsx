"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import {
  DRAWN_MAP_BYTES, fileProblem, firstFeatureProperties, guessNameProperty, HEAD_BYTES, MAX_MAP_BYTES, nameFromFile,
} from "@/lib/geojson-head"
import { mapService } from "@/services"
import { toServiceError } from "@/services/errors"
import type { MapUser, UploadedMap } from "@/services/contracts/maps"

const mb = (bytes: number) => `${Math.round(bytes / 1024 / 1024)} MB`

/**
 * Admin > Maps (`BI-16` part C2b, plan T8): upload a GeoJSON boundary file (up
 * to 50 MB, simplified to 5 MB or less for drawing: owner decision Q4), name it
 * and choose the property that holds each region's name; rename or delete an
 * uploaded map. Anyone signed in sees the list; only a holder of
 * `settings:write` uploads, renames or deletes (the controls are not offered
 * otherwise; the server enforces it). The routes are not on the server yet
 * (T5 and T6); until then the list shows its read error.
 */
export function MapsPage() {
  const { hasPermission } = useAuth()
  return <MapsAdmin canWrite={hasPermission("settings:write")} />
}

export function MapsAdmin({ canWrite }: { canWrite: boolean }) {
  const state = useService((signal) => mapService.list(signal), [])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Maps"
        description="Your own boundaries (sales regions, districts) for region, pin and grid maps, used like a built-in map."
      />
      {canWrite ? <UploadForm onUploaded={state.reload} /> : <p className="text-sm text-muted-foreground">Only an administrator can upload or change maps.</p>}
      <SectionCard title="Uploaded maps">
        {state.status === "loading" ? <LoadingSkeleton /> : null}
        {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
        {state.status === "success" && state.data.length === 0 ? <EmptyState title="No map uploaded yet" className="py-4" /> : null}
        {state.status === "success" && state.data.length > 0 ? <MapList maps={state.data} canWrite={canWrite} onChanged={state.reload} /> : null}
      </SectionCard>
    </div>
  )
}

type Chosen = { file: File; keys: string[]; properties: Record<string, unknown> }

function UploadForm({ onUploaded }: { onUploaded: () => void }) {
  const [chosen, setChosen] = React.useState<Chosen | null>(null)
  const [name, setName] = React.useState("")
  const [nameProperty, setNameProperty] = React.useState("")
  const [problem, setProblem] = React.useState<string | null>(null)
  const [busy, setBusy] = React.useState(false)
  const [done, setDone] = React.useState<string | null>(null)
  const input = React.useRef<HTMLInputElement>(null)

  async function pick(file: File | undefined) {
    setDone(null)
    setChosen(null)
    setProblem(null)
    if (!file) return
    const wrong = fileProblem(file)
    if (wrong) return setProblem(wrong)
    const head = firstFeatureProperties(await file.slice(0, HEAD_BYTES).text())
    if (!head.ok) return setProblem(head.reason)
    if (head.keys.length === 0) return setProblem("The first feature has no properties to take a name from.")
    setChosen({ file, keys: head.keys, properties: head.properties })
    setName(nameFromFile(file.name))
    setNameProperty(guessNameProperty(head.properties))
  }

  async function upload() {
    if (!chosen || !name.trim() || !nameProperty) return
    setBusy(true)
    setProblem(null)
    try {
      const map = await mapService.upload({ file: chosen.file, name: name.trim(), nameProperty })
      setDone(`${map.name} uploaded: ${map.regionCount.toLocaleString("en-US")} regions.`)
      setChosen(null)
      setName("")
      if (input.current) input.current.value = ""
      onUploaded()
    } catch (err) {
      // The server's sentence ("The file has no polygons.") is fixed text, safe to show.
      setProblem(toServiceError(err).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <SectionCard title="Upload a map" description={`A GeoJSON FeatureCollection of polygons in WGS 84, up to ${mb(MAX_MAP_BYTES)}. It is simplified to ${mb(DRAWN_MAP_BYTES)} or less for drawing; the full boundaries are kept for finding which region a point falls in.`}>
      <div className="flex max-w-md flex-col gap-3">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="map-file">GeoJSON file</Label>
          <Input id="map-file" ref={input} type="file" accept=".geojson,.json,application/geo+json,application/json" onChange={(e) => void pick(e.target.files?.[0])} disabled={busy} />
        </div>
        {chosen ? (
          <>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="map-name">Name</Label>
              <Input id="map-name" value={name} onChange={(e) => setName(e.target.value)} maxLength={80} disabled={busy} />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label>Property that holds the region name</Label>
              <Select value={nameProperty} items={Object.fromEntries(chosen.keys.map((k) => [k, k]))} onValueChange={(v) => setNameProperty(v ?? "")} disabled={busy}>
                <SelectTrigger className="w-full" aria-label="Name property"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {chosen.keys.map((k) => <SelectItem key={k} value={k}>{k}</SelectItem>)}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                First region: <span className="font-medium">{String(chosen.properties[nameProperty] ?? "(empty)")}</span>. Every region must have a different, non-empty name here.
              </p>
            </div>
            <Button className="w-fit" onClick={() => void upload()} disabled={busy || !name.trim() || !nameProperty}>
              {busy ? "Uploading…" : "Upload map"}
            </Button>
          </>
        ) : null}
        {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
        {done ? <p role="status" className="text-sm text-muted-foreground">{done}</p> : null}
      </div>
    </SectionCard>
  )
}

function MapList({ maps, canWrite, onChanged }: { maps: UploadedMap[]; canWrite: boolean; onChanged: () => void }) {
  const [renaming, setRenaming] = React.useState<{ id: string; name: string } | null>(null)
  const [removing, setRemoving] = React.useState<UploadedMap | null>(null)
  const [busy, setBusy] = React.useState(false)
  const [message, setMessage] = React.useState<string | null>(null)
  const [users, setUsers] = React.useState<MapUser[]>([])

  async function rename() {
    if (!renaming || !renaming.name.trim()) return
    setBusy(true)
    setMessage(null)
    try {
      await mapService.rename(renaming.id, renaming.name.trim())
      setRenaming(null)
      onChanged()
    } catch (err) {
      setMessage(toServiceError(err).message)
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    if (!removing) return
    setBusy(true)
    setMessage(null)
    setUsers([])
    try {
      const result = await mapService.remove(removing.id)
      if (result.ok) {
        setRemoving(null)
        onChanged()
      } else {
        setMessage(result.message)
        setUsers(result.charts)
      }
    } catch (err) {
      setMessage(toServiceError(err).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <ul className="divide-y divide-border text-sm">
        {maps.map((m) => (
          <li key={m.id} className="flex flex-wrap items-center gap-x-4 gap-y-1 py-2">
            {renaming?.id === m.id ? (
              <>
                <Input className="h-8 w-56" aria-label={`New name for ${m.name}`} value={renaming.name} onChange={(e) => setRenaming({ id: m.id, name: e.target.value })} maxLength={80} />
                <Button size="sm" onClick={() => void rename()} disabled={busy || !renaming.name.trim()}>Save</Button>
                <Button size="sm" variant="ghost" onClick={() => setRenaming(null)} disabled={busy}>Cancel</Button>
              </>
            ) : (
              <>
                <span className="min-w-0 flex-1 font-medium">{m.name}</span>
                <span className="text-muted-foreground">{m.regionCount.toLocaleString("en-US")} regions, names from “{m.nameProperty}”</span>
                {canWrite ? (
                  <>
                    <Button size="sm" variant="ghost" aria-label={`Rename ${m.name}`} onClick={() => { setMessage(null); setRenaming({ id: m.id, name: m.name }) }}>Rename</Button>
                    <Button size="sm" variant="ghost" aria-label={`Delete ${m.name}`} onClick={() => { setMessage(null); setUsers([]); setRemoving(m) }}>Delete</Button>
                  </>
                ) : null}
              </>
            )}
          </li>
        ))}
      </ul>
      {message && !removing ? <p role="alert" className="mt-2 text-sm text-destructive">{message}</p> : null}
      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(open) => { if (!open) setRemoving(null) }}
        title={`Delete ${removing?.name ?? "map"}`}
        description="The map is removed for everyone. A map that a chart uses is not deleted."
        confirmLabel="Delete map"
        confirming={busy}
        destructive
        onConfirm={() => void remove()}
      >
        {message ? <p role="alert" className="text-sm text-destructive">{message}</p> : null}
        {users.length > 0 ? (
          <ul aria-label="Charts that use this map" className="list-disc pl-5 text-sm">
            {users.map((u) => <li key={u.chartId}>{u.chartTitle} <span className="text-muted-foreground">on {u.dashboardTitle}</span></li>)}
          </ul>
        ) : null}
      </ConfirmActionDialog>
    </>
  )
}
