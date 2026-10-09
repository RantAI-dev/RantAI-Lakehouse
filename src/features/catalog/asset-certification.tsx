"use client"

import * as React from "react"
import Link from "next/link"
import { BadgeCheck } from "lucide-react"
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
import { useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { assetService } from "@/services"
import type { Asset, AssetCertificationInput } from "@/services/contracts/assets"

/** The API's bound on a deprecation note (`routes::catalog::put_certification`). */
const NOTE_MAX = 1000

type Choice = "none" | "certified" | "deprecated"

/** The choice an asset's current mark stands for. */
function choiceOf(certification: string | undefined): Choice {
  return certification === "certified" || certification === "deprecated" ? certification : "none"
}

const CHOICES: { value: Choice; label: string; note: string }[] = [
  { value: "none", label: "No mark", note: "The table carries no mark." },
  { value: "certified", label: "Certified", note: "Vouch for this table as the one to trust." },
  {
    value: "deprecated",
    label: "Deprecated",
    note: "Tell people to stop using it, and say what to use instead.",
  },
]

/**
 * On a deprecated asset's page: says so, with the reason and a link to the
 * replacement when there is one (`DATA-12`). Renders nothing otherwise. The
 * style is `StandInNotice`'s: the page has no shared alert component.
 */
export function DeprecationNotice({
  asset,
}: {
  asset: Pick<Asset, "certification" | "certificationNote" | "replacementAssetId">
}) {
  if (asset.certification !== "deprecated") return null
  return (
    <p
      role="note"
      className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400"
    >
      <span className="font-medium">This table is deprecated.</span>
      {asset.certificationNote ? <> {asset.certificationNote}</> : null}
      {asset.replacementAssetId ? (
        <>
          {" "}
          Use{" "}
          <Link
            href={`/data/assets/${asset.replacementAssetId}`}
            className="font-mono underline underline-offset-2"
          >
            {asset.replacementAssetId}
          </Link>{" "}
          instead.
        </>
      ) : null}
    </p>
  )
}

function CertificationDialog({
  asset: a,
  onClose,
  onSaved,
}: {
  asset: Pick<Asset, "id" | "name" | "certification" | "certificationNote" | "replacementAssetId">
  onClose: () => void
  onSaved: () => void
}) {
  const [choice, setChoice] = React.useState<Choice>(choiceOf(a.certification))
  const [note, setNote] = React.useState(a.certificationNote ?? "")
  const [replacement, setReplacement] = React.useState(a.replacementAssetId ?? "")
  const save = useServiceAction((signal, input: AssetCertificationInput) => {
    if (!assetService.setCertification) {
      return Promise.reject(new Error("This deployment cannot set a certification."))
    }
    return assetService.setCertification(a.id, input, signal)
  })

  async function submit() {
    const input: AssetCertificationInput =
      choice === "none"
        ? { status: null }
        : choice === "certified"
          ? { status: "certified" }
          : {
              status: "deprecated",
              // Blank is "not given": the API reads a blank as absent too.
              ...(note.trim() ? { note: note.trim() } : {}),
              ...(replacement.trim() ? { replacementAssetId: replacement.trim() } : {}),
            }
    const ok = await save.run(input)
    // `run` resolves to `null` only on failure, which `save.error` shows
    // with the API's own sentence; the dialog stays open.
    if (ok === null) return
    notifySuccess("Certification saved")
    onSaved()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Certification</DialogTitle>
          <DialogDescription>
            Mark {a.name} as the table to trust, or as one to stop using. The mark shows in
            search, the Data Explorer and the page header.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <fieldset className="grid gap-2">
            <legend className="sr-only">Mark</legend>
            {CHOICES.map((option) => (
              <div key={option.value}>
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input
                    type="radio"
                    name="asset-certification"
                    value={option.value}
                    checked={choice === option.value}
                    aria-describedby={`asset-certification-${option.value}`}
                    onChange={() => setChoice(option.value)}
                  />
                  {option.label}
                </label>
                <p
                  id={`asset-certification-${option.value}`}
                  className="mt-0.5 pl-6 text-xs text-muted-foreground"
                >
                  {option.note}
                </p>
              </div>
            ))}
          </fieldset>
          {choice === "deprecated" ? (
            <>
              <div className="grid gap-1.5">
                <Label htmlFor="asset-certification-note">Note</Label>
                <Textarea
                  id="asset-certification-note"
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  placeholder="Why it is deprecated"
                  maxLength={NOTE_MAX}
                  rows={3}
                />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="asset-certification-replacement">Replacement asset id</Label>
                <Input
                  id="asset-certification-replacement"
                  value={replacement}
                  onChange={(e) => setReplacement(e.target.value)}
                  placeholder="serving.monthly_orders"
                  maxLength={200}
                />
              </div>
            </>
          ) : null}
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" onClick={() => void submit()} disabled={save.status === "pending"}>
            {save.status === "pending" ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * The "Certification" button and its dialog. The caller shows it only to a
 * holder of `governance:write`; the API enforces the same permission.
 */
export function CertificationAction({
  asset,
  onChanged,
}: {
  asset: Pick<Asset, "id" | "name" | "certification" | "certificationNote" | "replacementAssetId">
  /** Called after a save, to reload the asset. */
  onChanged: () => void
}) {
  const [open, setOpen] = React.useState(false)
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <BadgeCheck />
        Certification
      </Button>
      {open ? (
        // Mounted only while open, so each opening starts from the saved mark.
        <CertificationDialog
          asset={asset}
          onClose={() => setOpen(false)}
          onSaved={() => {
            setOpen(false)
            onChanged()
          }}
        />
      ) : null}
    </>
  )
}
