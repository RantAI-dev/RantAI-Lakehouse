"use client"

import * as React from "react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import {
  brandingImageUrl,
  brandingService,
  settingsService,
  type BrandingImageKind,
  type WorkspaceSettingsInput,
} from "@/services"

/**
 * The longest name the server accepts (`ADM-2` decision 4). The field itself
 * refuses to hold more, so a name that reaches the server is one it accepts.
 */
const NAME_MAX = 60

/** Which of the three pictures a row is about. */
type PictureKey = "logo" | "logoDark" | "icon"

/**
 * The three pictures and the rule each one follows (`ADM-2` decision 3). The
 * server reads the type from the file's first bytes and refuses a file that
 * breaks its rule with a sentence of its own; the hint here only says what
 * the server accepts.
 */
const PICTURES: {
  kind: BrandingImageKind
  key: PictureKey
  label: string
  hint: string
  accept: string
}[] = [
  {
    kind: "logo",
    key: "logo",
    label: "Logo",
    hint: "PNG, JPEG or WebP, at most 512 KB.",
    accept: "image/png,image/jpeg,image/webp",
  },
  {
    kind: "logo-dark",
    key: "logoDark",
    label: "Dark-mode logo",
    hint: "PNG, JPEG or WebP, at most 512 KB.",
    accept: "image/png,image/jpeg,image/webp",
  },
  {
    kind: "icon",
    key: "icon",
    label: "Browser-tab icon",
    hint: "PNG, at most 64 KB.",
    accept: "image/png",
  },
]

/**
 * Admin > Settings > General (`ADM-2`, plan decision 1): the workspace's own
 * name, logo, dark-mode logo and browser-tab icon.
 *
 * The tab is offered to a holder of `settings:write` only, so every control
 * here is a write; the server refuses the rest anyway (fail closed). The
 * pictures are shown through the public branding route, which serves the
 * bytes with the type they were stored with.
 *
 * A refusal is the server's own sentence and is shown as it is: the console
 * does not rewrite what the server said about a file or a name.
 */
export function GeneralSettings() {
  const state = useService((signal) => settingsService.getGeneral(signal), [])
  const branding = useService((signal) => brandingService.get(signal), [])
  const [name, setName] = React.useState("")
  const [asking, setAsking] = React.useState(false)

  const save = useServiceAction(
    withNotify(
      { success: "The workspace name was saved", error: "The workspace name was not saved" },
      (signal: AbortSignal, input: WorkspaceSettingsInput) =>
        settingsService.saveGeneralName(input, signal)
    )
  )
  const upload = useServiceAction(
    withNotify(
      { success: "The picture was stored", error: "The picture was not stored" },
      (signal: AbortSignal, kind: BrandingImageKind, file: File) =>
        settingsService.uploadGeneralImage(kind, file, signal)
    )
  )
  const remove = useServiceAction(
    withNotify(
      { success: "The picture was removed", error: "The picture was not removed" },
      (signal: AbortSignal, kind: BrandingImageKind) =>
        settingsService.removeGeneralImage(kind, signal)
    )
  )
  const reset = useServiceAction(
    withNotify(
      { success: "The workspace settings were reset", error: "The workspace settings were not reset" },
      (signal: AbortSignal) => settingsService.resetGeneral(signal)
    )
  )

  const loaded = state.status === "success" ? state.data : null
  const publicView = branding.status === "success" ? branding.data : null

  React.useEffect(() => {
    if (loaded) setName(loaded.workspaceName ?? "")
  }, [loaded])

  async function handleSave() {
    const text = name.trim()
    const result = await save.run({ workspaceName: text === "" ? null : text })
    if (result) state.reload()
  }

  async function uploadPicture(kind: BrandingImageKind, file: File) {
    const result = await upload.run(kind, file)
    if (result) {
      state.reload()
      branding.reload()
    }
  }

  async function removePicture(kind: BrandingImageKind) {
    const result = await remove.run(kind)
    if (result) {
      state.reload()
      branding.reload()
    }
  }

  async function confirmReset() {
    const result = await reset.run()
    if (result) {
      setAsking(false)
      state.reload()
      branding.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="General"
        description="The workspace's own name, logo and browser-tab icon. Everybody sees them from the next page load."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {loaded ? (
        <div className="flex max-w-2xl flex-col gap-6">
          <section className="flex flex-col gap-1.5" aria-labelledby="general-name">
            <h2 id="general-name" className="text-sm font-medium">
              Name
            </h2>
            <Label htmlFor="workspace-name">Workspace name</Label>
            <Input
              id="workspace-name"
              maxLength={NAME_MAX}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Rantai Lake"
              autoComplete="off"
            />
            <p className="text-xs text-muted-foreground">
              At most 60 characters. An empty name means the product&apos;s own name.
            </p>
            <div className="flex items-center gap-3 pt-1">
              <Button className="w-fit" onClick={handleSave} disabled={save.status === "pending"}>
                Save
              </Button>
            </div>
            {save.error ? (
              <p role="alert" className="text-xs text-destructive">
                {save.error.message}
              </p>
            ) : null}
          </section>
          <section className="flex flex-col gap-4" aria-labelledby="general-pictures">
            <h2 id="general-pictures" className="text-sm font-medium">
              Pictures
            </h2>
            {PICTURES.map((picture) => {
              const summary = loaded[picture.key]
              const version = publicView?.[picture.key]?.version
              return (
                <div key={picture.kind} className="flex flex-col gap-1.5">
                  <Label htmlFor={`general-${picture.kind}`}>{picture.label}</Label>
                  <div className="flex items-center gap-3">
                    {version ? (
                      // eslint-disable-next-line @next/next/no-img-element -- the picture is served by the API at /api/branding/images/<kind>, not a file under `public/`; `next/image` would proxy it through the console's image optimizer.
                      <img
                        src={brandingImageUrl(picture.kind, version)}
                        alt={picture.label}
                        className="size-10 rounded border object-contain"
                      />
                    ) : (
                      <span className="text-xs text-muted-foreground">No picture set.</span>
                    )}
                    <Input
                      id={`general-${picture.kind}`}
                      type="file"
                      accept={picture.accept}
                      className="w-auto"
                      onChange={(e) => {
                        const file = e.target.files?.[0]
                        if (file) void uploadPicture(picture.kind, file)
                      }}
                    />
                    <Button
                      variant="ghost"
                      aria-label={`Remove the ${picture.label.toLowerCase()}`}
                      disabled={!summary.set || remove.status === "pending"}
                      onClick={() => void removePicture(picture.kind)}
                    >
                      Remove
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">{picture.hint}</p>
                </div>
              )
            })}
            {upload.error ? (
              <p role="alert" className="text-xs text-destructive">
                {upload.error.message}
              </p>
            ) : null}
            {remove.error ? (
              <p role="alert" className="text-xs text-destructive">
                {remove.error.message}
              </p>
            ) : null}
          </section>
          <section className="flex flex-col gap-1.5" aria-labelledby="general-reset">
            <h2 id="general-reset" className="text-sm font-medium">
              Reset
            </h2>
            <p className="text-xs text-muted-foreground">
              The workspace goes back to the product&apos;s own name, logo and icon.
            </p>
            <div className="flex items-center gap-3 pt-1">
              <Button variant="outline" onClick={() => setAsking(true)}>
                Reset
              </Button>
            </div>
            {reset.error ? (
              <p role="alert" className="text-xs text-destructive">
                {reset.error.message}
              </p>
            ) : null}
          </section>
        </div>
      ) : null}
      <ConfirmActionDialog
        open={asking}
        onOpenChange={setAsking}
        title="Reset the workspace settings"
        description="The workspace goes back to the product's own name, logo and icon."
        impact="The name and the three pictures are cleared. Nothing else is touched."
        confirmLabel="Reset everything"
        confirming={reset.status === "pending"}
        destructive
        onConfirm={() => void confirmReset()}
      />
    </div>
  )
}
