"use client"

import * as React from "react"
import { Pencil } from "lucide-react"
import { MetadataList } from "@/components/patterns/metadata-list"
import { SelectField } from "@/components/patterns/select-field"
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
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { CLASSIFICATION_LABEL } from "@/lib/status"
import { cn } from "@/lib/utils"
import { assetService, tagKeysService } from "@/services"
import type { AssetAnnotation, AssetDetail } from "@/services/contracts/assets"
import { SENSITIVITY_LEVELS, SensitivityControls } from "./column-sensitivity"
import { chosenGovernedTags, governedPairLabels } from "./governed-tags"

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
  // DATA-12 part 2: one select per defined key. The keys load with the dialog;
  // until they have, if they cannot be read, and while none is defined, no
  // `governedTags` is sent, so a save never wipes values the editor could not
  // show.
  const tagKeys = useService((s) => tagKeysService.list(s), [])
  const [governed, setGoverned] = React.useState<Record<string, string>>(
    a.annotation?.governedTags ?? {}
  )
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
      ...(tagKeys.status === "success" && tagKeys.data.length > 0
        ? { governedTags: chosenGovernedTags(governed) }
        : {}),
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
          {tagKeys.status === "success" && tagKeys.data.length > 0 ? (
            <div className="grid gap-1.5">
              <Label>Governed tags</Label>
              <div className="grid gap-2 sm:grid-cols-2">
                {tagKeys.data.map((tagKey) => (
                  <div key={tagKey.key} className="grid gap-1">
                    <Label htmlFor={`asset-governed-${tagKey.key}`} className="font-mono text-xs">
                      {tagKey.key}
                    </Label>
                    <SelectField
                      id={`asset-governed-${tagKey.key}`}
                      className="w-full"
                      value={governed[tagKey.key] ?? ""}
                      options={[
                        /* WEB-9 D2: "None" is a real choice, not a prompt. */
                        { value: "", label: "None" },
                        ...tagKey.allowedValues.map((value) => ({ value, label: value })),
                      ]}
                      onValueChange={(next) =>
                        setGoverned({ ...governed, [tagKey.key]: next })
                      }
                    />
                  </div>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                Values an admin agreed on. Only these can be chosen; the free tags above are not
                affected.
              </p>
            </div>
          ) : null}
          {tagKeys.status === "error" ? (
            <p className="text-xs text-muted-foreground">
              The governed tag keys could not be loaded, so they are left as they are.
            </p>
          ) : null}
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
 * Why the header badge is stricter than the table-level rule, or `null`
 * when it is not. A table is never less strict than its strictest column
 * (review round 1 decision 1), so a column rule can raise the header above
 * what the whole-table rule says; the card names those columns rather than
 * leave two levels on one screen unexplained.
 */
export function stricterBecauseOfColumns(a: AssetDetail): string | null {
  const rank = (l: AssetDetail["classification"] | undefined) =>
    l ? SENSITIVITY_LEVELS.indexOf(l) : -1
  const tableRule = (a.classificationRules ?? []).find((r) => r.column === undefined)
  const strictest = a.schema.filter(
    (c) => c.classification === a.classification && rank(c.classification) > rank(tableRule?.classification)
  )
  if (strictest.length === 0) return null
  const names = strictest.map((c) => c.name).join(", ")
  const level = CLASSIFICATION_LABEL[a.classification]
  return strictest.length === 1
    ? `The header shows ${level} because column ${names} is ${level}: a table is never less restrictive than its most restrictive column.`
    : `The header shows ${level} because columns ${names} are ${level}: a table is never less restrictive than its most restrictive column.`
}

/**
 * What the asset is and who answers for it. Description, owner, steward,
 * tags and governed tags can be edited here by anyone with `catalog:write`; the rest is
 * the registry's. The whole table's sensitivity (its table-level
 * classification rule) is set here too, with `governance:write`.
 */
export function AssetAbout({
  asset: a,
  schemaLabel,
  onChanged,
}: {
  asset: AssetDetail
  /**
   * The current schema version, worked out by the overview: an Iceberg
   * table's own, or the one the console recorded for a Silver or Gold
   * table; `null` when there is none yet.
   */
  schemaLabel: string | null
  /** Called after a save, to reload the asset. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const [editing, setEditing] = React.useState(false)
  const tags = a.tags ?? []
  const governedPairs = governedPairLabels(a.governedTags)
  const tableRule = (a.classificationRules ?? []).find((r) => r.column === undefined)
  const stricterNote = stricterBecauseOfColumns(a)

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
      {/* The description itself is in the page header, above every tab;
          repeating it here said the same sentence twice on one screen. */}
      {a.description ? null : (
        <p className="mb-3 text-sm text-muted-foreground">
          No description yet. Ask the owner to document what one row represents.
        </p>
      )}
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
          ...(schemaLabel ? [{ label: "Schema", value: schemaLabel }] : []),
          // Governed pairs (`key: value`, tone "info") sit beside the free tags
          // (neutral) in one row: both are how the asset is found.
          ...(tags.length > 0 || governedPairs.length > 0
            ? [
                {
                  label: "Tags",
                  value: (
                    <span className="flex flex-wrap gap-1">
                      {governedPairs.map((pair) => (
                        <Pill key={`governed:${pair}`} tone="info" title="Governed tag">
                          {pair}
                        </Pill>
                      ))}
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
      {/* WEB-3 S3-T5a (review round 1 decision 1): the whole table's own
          classification rule, which the Access tab's Classify dialog used to
          set (S3-D2 removed it). A column's level is set on the column. */}
      <section
        aria-label="Sensitivity (whole table)"
        className="mt-3 grid gap-1.5 border-t border-border pt-3 sm:grid-cols-[10rem_minmax(0,1fr)] sm:gap-3"
      >
        <div className="text-xs font-medium text-muted-foreground sm:pt-1.5">Sensitivity (whole table)</div>
        <div className="flex min-w-0 flex-col gap-2">
          <SensitivityControls
            asset={a}
            level={tableRule?.classification}
            canWrite={hasPermission("governance:write")}
            onChanged={onChanged}
          />
          {stricterNote ? <p className="text-xs text-muted-foreground">{stricterNote}</p> : null}
        </div>
      </section>
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
