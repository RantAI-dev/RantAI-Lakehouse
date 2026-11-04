"use client"

import * as React from "react"
import Link from "next/link"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { Button } from "@/components/ui/button"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { notifyError, notifySuccess } from "@/lib/notify"
import { governanceService } from "@/services"

/** What happened to the policy, for the caller to refresh what it shows. */
export type PolicyChange = "enforced" | "stopped" | "deleted"

type Action = "enforce" | "stop" | "delete"

/**
 * Why a policy cannot be enforced, in one sentence (`SEC-61`). Said on the
 * disabled Enforce / Stop enforcing buttons and in the drawer: the engine
 * applies only a structured `conditions` clause, so switching a policy
 * written as text on changes nothing.
 */
export const POLICY_NOT_ENFORCEABLE =
  "This policy is written as text, so it masks nothing and filters no rows. Add the rule on the table's Access tab, where it is enforced, then delete this one."

/**
 * The drawer's line for a policy the engine cannot enforce (`SEC-61`),
 * above its actions: says plainly that it does nothing, and links to the
 * catalog, where the table's Access tab writes a rule the engine applies.
 */
export function PolicyNotEnforceableNotice() {
  return (
    <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
      {POLICY_NOT_ENFORCEABLE}{" "}
      <Link href="/catalog" className="font-medium underline underline-offset-2">
        Open the catalog
      </Link>
    </p>
  )
}

/**
 * What the confirmation says before each action. Stopping or deleting an
 * enforced policy takes protection away, so those say so; deleting a
 * draft changes nothing that is enforced, and says that instead.
 */
export function policyConfirmation(action: Action, policy: { name: string; status: string }) {
  const lifts = "From the next query on, whatever it masks or filters is read unmasked and unfiltered."
  switch (action) {
    case "enforce":
      return {
        title: "Enforce policy",
        description: `Enforce ${policy.name}?`,
        impact: "From the next query on, the roles it names read its masked columns as *** and only the rows its filter allows.",
        confirmLabel: "Enforce",
        destructive: false,
        done: `Policy ${policy.name} is enforced`,
      }
    case "stop":
      return {
        title: "Stop enforcing policy",
        description: `Stop enforcing ${policy.name}?`,
        impact: `${lifts} The policy is kept as a draft.`,
        confirmLabel: "Stop enforcing",
        destructive: true,
        done: `Policy ${policy.name} is no longer enforced`,
      }
    case "delete":
      return {
        title: "Delete policy",
        description: `Delete ${policy.name}?`,
        impact:
          policy.status === "ready"
            ? `It is being enforced. ${lifts}`
            : "It is a draft, so nothing that is enforced changes.",
        confirmLabel: "Delete policy",
        destructive: true,
        done: `Deleted policy ${policy.name}`,
      }
  }
}

/**
 * Enforce / stop enforcing / delete for one policy, each behind a
 * confirmation that says what it changes. Renders nothing for someone
 * without `policy:write`.
 *
 * Enforce and Stop enforcing are disabled when the engine cannot enforce
 * the policy (`SEC-61`): switching such a policy on changes nothing, and
 * the button says why instead of pretending otherwise. Delete stays, since
 * removing a policy that enforces nothing is the honest next step.
 */
export function PolicyActions({
  policy,
  onChanged,
}: {
  policy: { id: string; name: string; status: string; enforceable: boolean }
  onChanged: (change: PolicyChange) => void
}) {
  const { hasPermission } = useAuth()
  const [asking, setAsking] = React.useState<Action | null>(null)
  const run = useServiceAction(async (signal, action: Action) => {
    if (action === "delete") await governanceService.deletePolicy(policy.id, signal)
    else await governanceService.setPolicyStatus(policy.id, action === "enforce" ? "ready" : "draft", signal)
    return action
  })
  if (!hasPermission("policy:write")) return null

  const enforced = policy.status === "ready"
  const confirmation = asking ? policyConfirmation(asking, policy) : null

  async function confirm() {
    if (!asking || !confirmation) return
    const done = await run.run(asking)
    if (done === null) {
      notifyError(`${confirmation.title} failed`, run.error)
      return
    }
    notifySuccess(confirmation.done)
    setAsking(null)
    onChanged(done === "enforce" ? "enforced" : done === "stop" ? "stopped" : "deleted")
  }

  return (
    <>
      <Button
        size="sm"
        variant="outline"
        disabled={!policy.enforceable}
        title={policy.enforceable ? undefined : POLICY_NOT_ENFORCEABLE}
        onClick={() => setAsking(enforced ? "stop" : "enforce")}
      >
        {enforced ? "Stop enforcing" : "Enforce"}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        aria-label={`Delete policy ${policy.name}`}
        onClick={() => setAsking("delete")}
      >
        Delete
      </Button>
      {confirmation ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => (open ? undefined : setAsking(null))}
          title={confirmation.title}
          description={confirmation.description}
          impact={confirmation.impact}
          confirmLabel={confirmation.confirmLabel}
          confirming={run.status === "pending"}
          destructive={confirmation.destructive}
          onConfirm={() => void confirm()}
        />
      ) : null}
    </>
  )
}
