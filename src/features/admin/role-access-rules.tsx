"use client"

import * as React from "react"
import Link from "next/link"

import { LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useAuth } from "@/features/auth/auth-provider"
import { accessRule, ruleClause } from "@/features/governance/access-rules"
import { AccessRuleTable, PolicyStatusPill } from "@/features/governance/policy-columns"
import { useService } from "@/hooks/use-service"
import { governanceService } from "@/services"

/**
 * The `GOV-9` D4 section on Edit role: the masks and row filters that name
 * this role, read-only. They are still made, enforced, stopped and deleted
 * on a table's Access tab (`WEB-3` S3-D4); this section only shows them and
 * links there, so a role's data access reads in one place without a second
 * editor for the same fact.
 *
 * A rule is this role's only when the engine would apply it to the role, so
 * the list is built from `ruleClause` — the API's `enforceable` clause —
 * never from the rule's free-text `subjects`. The match on the name is exact
 * and case-sensitive, as `condition_names_role` in
 * `rust/crates/lakehouse-api/src/policy_engine.rs` matches it. A rule the
 * API did not call enforceable is skipped by the engine, so it is not
 * listed: it names no role in a form that can be matched.
 */
export function RoleAccessRulesSection({ roleName }: { roleName: string }) {
  const { hasPermission } = useAuth()
  const canRead = hasPermission("policy:read")
  // `GOV-9` D4: without `policy:read` the route answers 403, so the fetcher
  // resolves to an empty list and sends no request at all.
  const state = useService(
    (signal) => (canRead ? governanceService.listPolicies(signal) : Promise.resolve([])),
    [canRead]
  )

  const rules = React.useMemo(() => {
    if (state.status !== "success") return []
    return state.data.flatMap((policy) => {
      const clause = ruleClause(policy)
      return clause && clause.roles.includes(roleName) ? [accessRule(policy)] : []
    })
  }, [state.status, state.data, roleName])

  return (
    <section className="space-y-3" aria-labelledby="role-access-rules-title">
      <div className="space-y-1">
        <h2 id="role-access-rules-title" className="text-sm font-semibold">
          Masks and row filters on this role
        </h2>
        <p className="text-xs text-muted-foreground">
          A rule is made, enforced, stopped and deleted on the Access tab of the table it
          governs.
        </p>
      </div>

      {!canRead ? (
        <p className="text-xs text-muted-foreground">
          Seeing the masks and row filters on a role needs the permission policy:read.
        </p>
      ) : state.status === "loading" ? (
        <LoadingSkeleton rows={2} />
      ) : state.status === "error" ? (
        <div className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-3">
          <p role="alert" className="text-sm text-destructive">
            The masks and row filters on this role could not be loaded:{" "}
            {state.error.message}
          </p>
          <Button type="button" variant="outline" size="sm" onClick={state.reload}>
            Retry
          </Button>
        </div>
      ) : rules.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          No mask or row filter names this role. They are added on a table&apos;s Access tab.
        </p>
      ) : (
        <ul className="divide-y divide-border rounded-md border">
          {rules.map((rule) => (
            <li
              key={rule.id}
              className="flex items-start justify-between gap-3 p-2.5"
            >
              <div className="min-w-0 space-y-0.5">
                <AccessRuleTable rule={rule} />
                <p className="text-xs text-muted-foreground">{rule.does}</p>
              </div>
              <PolicyStatusPill policy={rule} />
            </li>
          ))}
        </ul>
      )}

      {canRead && state.status === "success" ? (
        // `GOV-9` D4: a plain-text rule is skipped by the engine, so it
        // cannot name this role in a matchable form. Saying so stops it
        // reading as a rule that went missing.
        <p className="text-xs text-muted-foreground">
          A rule written as plain text is not listed: the engine skips it, so it names no
          role.{" "}
          <Link
            href="/governance/policies"
            className="text-primary underline-offset-2 hover:underline"
          >
            Open access rules
          </Link>
          .
        </p>
      ) : null}
    </section>
  )
}
