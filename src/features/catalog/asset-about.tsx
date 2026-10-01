"use client"

import * as React from "react"
import { Pencil } from "lucide-react"
import { MetadataList } from "@/components/patterns/metadata-list"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { assetService } from "@/services"
import type { AssetAnnotation, AssetDetail } from "@/services/contracts/assets"

/** The API's bounds on an annotation (`routes::catalog::validate_annotation_body`). */
const MAX_TAGS = 20
const TAG_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/

/**
 * Tags as typed — separated by commas or spaces — into the list the API
 * stores, or the first reason it would refuse them.
 */
export function parseTags(raw: string): { tags: string[] } | { error: string } {
  const tags = [...new Set(raw.split(/[\s,]+/).filter((t) => t.length > 0))]
  const bad = tags.find((t) => !TAG_PATTERN.test(t))
  if (bad) {
    return { error: `"${bad}" is not a valid tag: use lowercase letters, digits, "-" and "_".` }
  }
  if (tags.length > MAX_TAGS) return { error: `At most ${MAX_TAGS} tags.` }
  return { tags }
}

/** A blank field is stored as "not set", so the registry's value shows again. */
function orNull(value: string): string | null {
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

function EditDialog({
  asset: a,
  open,
  onClose,
  onSaved,
}: {
  asset: AssetDetail
  open: boolean
  onClose: () => void
  onSaved: () => void
}) {
  const [description, setDescription] = React.useState(a.annotation?.description ?? "")
  const [owner, setOwner] = React.useState(a.annotation?.owner ?? "")
  const [steward, setSteward] = React.useState(a.annotation?.steward ?? "")
  const [tags, setTags] = React.useState((a.annotation?.tags ?? []).join(", "))
  const save = useServiceAction((signal, input: AssetAnnotation) => {
    if (!assetService.updateAnnotation) {
      return Promise.reject(new Error("This deployment cannot edit asset details."))
    }
    return assetService.updateAnnotation(a.id, input, signal)
  })
  const parsed = parseTags(tags)

  async function submit() {
    if ("error" in parsed) return
    const ok = await save.run({
      description: orNull(description),
      owner: orNull(owner),
      steward: orNull(steward),
      tags: parsed.tags,
    })
    // `run` resolves to `null` only on failure, which `save.error` shows.
    if (ok === null) return
    notifySuccess("Asset details saved")
    onSaved()
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Edit details</DialogTitle>
          <DialogDescription>
            What people should know about {a.name}. An empty field goes back to what the
            registry says.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="asset-description">Description</Label>
            <Textarea
              id="asset-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={a.registry?.description || "What one row represents, and what it is for."}
              maxLength={4000}
              rows={4}
            />
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label htmlFor="asset-owner">Owner</Label>
              <Input
                id="asset-owner"
                value={owner}
                onChange={(e) => setOwner(e.target.value)}
                placeholder={a.registry?.owner || "Team or person accountable"}
                maxLength={128}
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="asset-steward">Steward</Label>
              <Input
                id="asset-steward"
                value={steward}
                onChange={(e) => setSteward(e.target.value)}
                placeholder="Who to ask about this data"
                maxLength={128}
              />
            </div>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="asset-tags">Tags</Label>
            <Input
              id="asset-tags"
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder="finance, pii, monthly-report"
              aria-invalid={"error" in parsed}
            />
            <p className={cn("text-xs", "error" in parsed ? "text-destructive" : "text-muted-foreground")}>
              {"error" in parsed ? parsed.error : "Separate with commas. Tags make the asset findable in search."}
            </p>
          </div>
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={"error" in parsed || save.status === "pending"}
          >
            {save.status === "pending" ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * What the asset is and who answers for it. Description, owner, steward
 * and tags can be edited here by anyone with `catalog:write`; the rest is
 * the registry's.
 */
export function AssetAbout({
  asset: a,
  schemaLabel,
  onChanged,
}: {
  asset: AssetDetail
  /** The current schema version, worked out by the overview. */
  schemaLabel: string
  /** Called after a save, to reload the asset. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const [editing, setEditing] = React.useState(false)
  const tags = a.tags ?? []

  return (
    <SectionCard
      size="sm"
      title="About"
      action={
        hasPermission("catalog:write") ? (
          <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
            <Pencil />
            Edit
          </Button>
        ) : undefined
      }
    >
      <p className={cn("mb-3 text-sm", !a.description && "text-muted-foreground")}>
        {a.description || "No description yet. Ask the owner to document what one row represents."}
      </p>
      <MetadataList
        density="compact"
        columns={2}
        items={[
          { label: "Domain", value: a.domain || "—" },
          { label: "Owner", value: a.owner || "—" },
          ...(a.steward ? [{ label: "Steward", value: a.steward }] : []),
          { label: "Update frequency", value: a._meta?.frekuensi || "—" },
          ...(a._meta?.satuan ? [{ label: "Unit", value: a._meta.satuan }] : []),
          ...(a._meta?.klasifikasi
            ? [{ label: "Publisher classification", value: a._meta.klasifikasi }]
            : []),
          { label: "Columns", value: String(a.schema.length || a.columnCount) },
          { label: "Schema", value: schemaLabel },
          ...(tags.length > 0
            ? [
                {
                  label: "Tags",
                  value: (
                    <span className="flex flex-wrap gap-1">
                      {tags.map((tag) => (
                        <Pill key={tag} tone="neutral">
                          {tag}
                        </Pill>
                      ))}
                    </span>
                  ),
                },
              ]
            : []),
        ]}
      />
      {editing ? (
        // Mounted only while open, so each opening starts from the saved values.
        <EditDialog
          asset={a}
          open
          onClose={() => setEditing(false)}
          onSaved={() => {
            setEditing(false)
            onChanged()
          }}
        />
      ) : null}
    </SectionCard>
  )
}
