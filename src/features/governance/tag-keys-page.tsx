"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { notifySuccess, withNotify } from "@/lib/notify"
import { tagKeysService } from "@/services"
import type { TagKey } from "@/services/contracts/tag-keys"
import { formatTagValues, parseTagValues } from "./tag-key-values"

/**
 * Governed tag keys (`DATA-12` part 2): the keys an admin defines and the
 * values each may take. Anyone who can read the catalog sees the list; the
 * buttons that define, change and remove a key show only with
 * `governance:write`, as the API demands it. Every refusal shows the API's
 * own sentence, in the form it came from, so the screen never claims a save
 * the API refused.
 */
export function TagKeysPage() {
  const state = useService((s) => tagKeysService.list(s), [])
  const { hasPermission } = useAuth()
  const canWrite = hasPermission("governance:write")

  // One sheet serves both: `editing` is the key being changed, or null when
  // defining a new one.
  const [sheetOpen, setSheetOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<TagKey | null>(null)
  const [keyText, setKeyText] = React.useState("")
  const [valuesText, setValuesText] = React.useState("")
  const [removing, setRemoving] = React.useState<TagKey | null>(null)

  const save = useServiceAction(
    async (signal, input: { key: string; values: string[]; isNew: boolean }) =>
      input.isNew
        ? tagKeysService.create(input.key, input.values, signal)
        : tagKeysService.update(input.key, input.values, signal)
  )
  const remove = useServiceAction(
    withNotify(
      { success: "Tag key removed", error: "Could not remove the tag key" },
      (signal, key: string) => tagKeysService.remove(key, signal)
    )
  )

  function openNew() {
    setEditing(null)
    setKeyText("")
    setValuesText("")
    save.reset()
    setSheetOpen(true)
  }

  function openEdit(tagKey: TagKey) {
    setEditing(tagKey)
    setKeyText(tagKey.key)
    setValuesText(formatTagValues(tagKey.allowedValues))
    save.reset()
    setSheetOpen(true)
  }

  async function submit() {
    const saved = await save.run({
      key: editing ? editing.key : keyText.trim(),
      values: parseTagValues(valuesText),
      isNew: editing === null,
    })
    if (saved === null) return // the sheet shows the API's sentence
    notifySuccess(editing ? "Tag key updated" : "Tag key defined")
    setSheetOpen(false)
    state.reload()
  }

  async function confirmRemove() {
    if (!removing) return
    const done = await remove.run(removing.key)
    if (done === null) return // the dialog shows the API's sentence
    setRemoving(null)
    state.reload()
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Tag keys"
        description="Keys with agreed values, so a table is tagged data_domain: finance and not fin, finance or keuangan. Free one-word tags keep working beside them."
        actions={
          canWrite ? (
            <Button size="sm" onClick={openNew}>
              <PlusIcon data-icon="inline-start" />
              New tag key
            </Button>
          ) : null
        }
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && state.data.length === 0 ? (
        <EmptyState
          title="No tag keys"
          description={
            canWrite
              ? "Define a key and the values it may take, then give tables a value from a table's page."
              : "An admin with governance:write defines the keys."
          }
        />
      ) : null}
      {state.status === "success" && state.data.length > 0 ? (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Key</TableHead>
              <TableHead>Allowed values</TableHead>
              <TableHead>Defined by</TableHead>
              <TableHead>Changed</TableHead>
              {canWrite ? <TableHead className="text-right">Actions</TableHead> : null}
            </TableRow>
          </TableHeader>
          <TableBody>
            {state.data.map((tagKey) => (
              <TableRow key={tagKey.key}>
                <TableCell className="font-mono text-sm">{tagKey.key}</TableCell>
                <TableCell>
                  <div className="flex flex-wrap gap-1.5">
                    {tagKey.allowedValues.map((value) => (
                      <Pill key={value} tone="neutral">
                        {value}
                      </Pill>
                    ))}
                  </div>
                </TableCell>
                <TableCell>{tagKey.createdBy}</TableCell>
                <TableCell>{formatRelativeTime(tagKey.updatedAt)}</TableCell>
                {canWrite ? (
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        aria-label={`Edit values of ${tagKey.key}`}
                        onClick={() => openEdit(tagKey)}
                      >
                        Edit values
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Delete tag key ${tagKey.key}`}
                        onClick={() => {
                          remove.reset()
                          setRemoving(tagKey)
                        }}
                      >
                        Delete
                      </Button>
                    </div>
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}

      <CreateSheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        title={editing ? `Edit values of ${editing.key}` : "New tag key"}
        description={
          editing
            ? "A value some tables still have cannot be removed; the answer says how many."
            : "Lower-case letters, digits, _ and -, up to 64 characters. A key is never renamed."
        }
        submitLabel={editing ? "Save values" : "Define key"}
        submitting={save.status === "pending"}
        canSubmit={(editing !== null || keyText.trim() !== "") && valuesText.trim() !== ""}
        onSubmit={() => void submit()}
        error={save.error?.message ?? null}
      >
        <div className="space-y-1.5">
          <Label htmlFor="tk-key">Key</Label>
          <Input
            id="tk-key"
            value={keyText}
            disabled={editing !== null}
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
          <p className="text-xs text-muted-foreground">
            Separate with commas or spaces. At most 50.
          </p>
        </div>
      </CreateSheet>

      {removing ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => (open ? undefined : setRemoving(null))}
          title="Delete tag key"
          description={`Delete ${removing.key}?`}
          impact="Refused while any table still has a value for it; the answer says how many."
          confirmLabel="Delete tag key"
          confirming={remove.status === "pending"}
          destructive
          onConfirm={() => void confirmRemove()}
        >
          {remove.error ? (
            <p className="text-sm text-destructive">{remove.error.message}</p>
          ) : null}
        </ConfirmActionDialog>
      ) : null}
    </div>
  )
}
