"use client"

import * as React from "react"
import Link from "next/link"
import { useParams, useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { tagKeysService } from "@/services"
import { formatTagValues, parseTagValues } from "./tag-key-values"

/**
 * Where the tag-keys list lives (`WEB-3` S3-D6: a section of Settings), and
 * therefore where both form pages return to.
 */
const TAG_KEYS_HREF = "/admin/settings?section=tag-keys"

/**
 * `/admin/settings/tag-keys/new` (`WEB-7` D2): the define form, moved out
 * of the `CreateSheet` side panel onto its own page.
 *
 * The same two fields as before: the key (never renamed later) and the
 * values it may take, split with `parseTagValues`. A value the API would
 * refuse is sent as typed so the refusal is the API's own sentence.
 */
export function TagKeyCreatePage() {
  const router = useRouter()
  const [keyText, setKeyText] = React.useState("")
  const [valuesText, setValuesText] = React.useState("")
  const save = useServiceAction((signal, key: string, values: string[]) =>
    tagKeysService.create(key, values, signal)
  )

  async function submit() {
    const saved = await save.run(keyText.trim(), parseTagValues(valuesText))
    if (saved === null) return // the page shows the API's sentence
    notifySuccess("Tag key defined")
    router.push(TAG_KEYS_HREF)
  }

  return (
    <FormPage
      title="New tag key"
      description="Lower-case letters, digits, _ and -, up to 64 characters. A key is never renamed."
      backHref={TAG_KEYS_HREF}
      backLabel="Back to Tag keys"
      submitLabel="Define key"
      submittingLabel="Creating…"
      canSubmit={keyText.trim() !== "" && valuesText.trim() !== ""}
      submitting={save.status === "pending"}
      error={save.error?.message ?? null}
      onSubmit={() => void submit()}
      onCancel={() => router.push(TAG_KEYS_HREF)}
    >
      <div className="space-y-1.5">
        <Label htmlFor="tk-key">Key</Label>
        <Input
          id="tk-key"
          value={keyText}
          onChange={(e) => setKeyText(e.target.value)}
          placeholder="data_domain"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="tk-values">Allowed values</Label>
        <Input
          id="tk-values"
          value={valuesText}
          onChange={(e) => setValuesText(e.target.value)}
          placeholder="finance, ops, hr"
        />
        <p className="text-xs text-muted-foreground">Separate with commas or spaces. At most 50.</p>
      </div>
    </FormPage>
  )
}

/**
 * `/admin/settings/tag-keys/[key]/edit` (`WEB-7` D2/D4): the values form,
 * loaded by key through the existing list read so a reload works. A key
 * that no longer exists shows the not-found state.
 *
 * The key itself is shown and fixed: the API never renames one.
 */
export function TagKeyEditPage() {
  const { key } = useParams<{ key: string }>()
  const router = useRouter()
  const state = useService((s) => tagKeysService.list(s), [])
  const [valuesText, setValuesText] = React.useState<string | null>(null)
  const save = useServiceAction((signal, tagKey: string, values: string[]) =>
    tagKeysService.update(tagKey, values, signal)
  )

  const tagKey = state.status === "success" ? state.data.find((candidate) => candidate.key === key) : undefined

  // Fill the field once the key arrives; a refetch of the list must not
  // overwrite what the person has typed since.
  React.useEffect(() => {
    if (tagKey && valuesText === null) setValuesText(formatTagValues(tagKey.allowedValues))
  }, [tagKey, valuesText])

  if (state.status === "loading") return <LoadingSkeleton rows={6} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  if (!tagKey) {
    return (
      <EmptyState
        title="Tag key not found"
        description="This key was removed, or the address names one that never existed."
        action={
          <Button variant="outline" size="sm" render={<Link href={TAG_KEYS_HREF} />}>
            Back to Tag keys
          </Button>
        }
      />
    )
  }

  async function submit() {
    if (!tagKey) return
    const saved = await save.run(tagKey.key, parseTagValues(valuesText ?? ""))
    if (saved === null) return // the page shows the API's sentence
    notifySuccess("Tag key updated")
    router.push(TAG_KEYS_HREF)
  }

  return (
    <FormPage
      title={`Edit values of ${tagKey.key}`}
      description="A value some tables still have cannot be removed; the answer says how many."
      backHref={TAG_KEYS_HREF}
      backLabel="Back to Tag keys"
      submitLabel="Save values"
      submittingLabel="Saving…"
      canSubmit={(valuesText ?? "").trim() !== ""}
      submitting={save.status === "pending"}
      error={save.error?.message ?? null}
      onSubmit={() => void submit()}
      onCancel={() => router.push(TAG_KEYS_HREF)}
    >
      <div className="space-y-1.5">
        <Label htmlFor="tk-key">Key</Label>
        <Input id="tk-key" value={tagKey.key} disabled onChange={() => {}} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="tk-values">Allowed values</Label>
        <Input
          id="tk-values"
          value={valuesText ?? ""}
          onChange={(e) => setValuesText(e.target.value)}
          placeholder="finance, ops, hr"
        />
        <p className="text-xs text-muted-foreground">Separate with commas or spaces. At most 50.</p>
      </div>
    </FormPage>
  )
}
