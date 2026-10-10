"use client"

import { useState } from "react"
import Link from "next/link"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { connectorService } from "@/services"
import type { ConnectorDependent } from "@/services/contracts/connectors"
import { ServiceError } from "@/services/errors"

/**
 * Deleting a connector, with the two refusals the API can answer spelled
 * out instead of surfacing as a bare error:
 *
 * - Pipelines still read from it: the API refuses whatever `force` says, so
 *   this dialog does not even offer the delete — it lists the pipelines.
 * - A CDC replication slot could not be dropped (source unreachable): the
 *   API keeps the connector (409). The message is shown and a forced delete
 *   is offered, with the consequence stated, because a connector pointing
 *   at a decommissioned host could otherwise never be removed.
 */
export function ConnectorDeleteDialog({
  connector,
  dependents,
  open,
  onOpenChange,
  onDeleted,
}: {
  connector: { id: string; name: string }
  dependents: ConnectorDependent[]
  open: boolean
  onOpenChange: (open: boolean) => void
  onDeleted: () => void
}) {
  // Set once the user moves on to a forced delete, so the refusal that
  // prompted it stays on screen while the forced attempt runs.
  const [refusal, setRefusal] = useState<string | null>(null)
  const remove = useServiceAction((signal, force: boolean) =>
    connectorService.deleteConnector(connector.id, { force }, signal)
  )

  function close(next: boolean) {
    if (!next) {
      setRefusal(null)
      remove.reset()
    }
    onOpenChange(next)
  }

  async function run(force: boolean) {
    // `useServiceAction` resolves to `null` only on failure; `deleteConnector`
    // itself resolves with no value.
    if ((await remove.run(force)) !== null) {
      notifySuccess(`Connector "${connector.name}" deleted`)
      close(false)
      onDeleted()
    }
  }

  if (dependents.length > 0) {
    return (
      <Dialog open={open} onOpenChange={close}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Connector is still in use</DialogTitle>
            <DialogDescription>
              {connector.name} cannot be deleted while pipelines read from it. Delete those
              pipelines or point them at another connector first.
            </DialogDescription>
          </DialogHeader>
          <ul className="space-y-1 text-sm">
            {dependents.map((d) => (
              <li key={d.id}>
                <Link href={`/pipelines/${d.id}`} className="text-primary hover:underline">
                  {d.name}
                </Link>
              </li>
            ))}
          </ul>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => close(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    )
  }

  const error = remove.error
  const conflict =
    refusal ?? (error instanceof ServiceError && error.status === 409 ? error.message : null)

  if (conflict) {
    return (
      <ConfirmActionDialog
        open={open}
        onOpenChange={close}
        title="Connector was not deleted"
        description={conflict}
        impact="Force delete removes the connector anyway. Any replication slot and publication it created stay on the source database and keep retaining WAL until someone drops them by hand."
        confirmLabel="Force delete"
        destructive
        confirming={remove.status === "pending"}
        onConfirm={() => {
          setRefusal(conflict)
          void run(true)
        }}
      >
        {refusal && error ? <p className="text-sm text-destructive">{error.message}</p> : null}
      </ConfirmActionDialog>
    )
  }

  return (
    <ConfirmActionDialog
      open={open}
      onOpenChange={close}
      title={`Delete ${connector.name}?`}
      description="The connector and the credential stored for it are removed. This cannot be undone."
      impact="A CDC connector's replication slot and publication are dropped on the source database first."
      confirmLabel="Delete"
      destructive
      confirming={remove.status === "pending"}
      onConfirm={() => void run(false)}
    >
      {error ? <p className="text-sm text-destructive">{error.message}</p> : null}
    </ConfirmActionDialog>
  )
}
