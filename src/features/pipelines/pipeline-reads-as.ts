import type { PipelineDetail } from "@/services/contracts/pipelines"
import type { User } from "@/services/contracts/identity"

/**
 * Who a pipeline reads as (BLD-1 / D2), and what a Platform Admin may
 * offer as an author.
 *
 * The page shows this in one place and needs the same three answers in
 * three shapes — a chip saying who it reads as, an empty state explaining
 * why nothing runs, and the option list behind the assign control — so the
 * decisions live here where they can be tested, rather than inline in JSX.
 */

/**
 * The refusal the API returns, and the sentence the page shows. One
 * string, copied from `routes::authored_pipelines::NO_AUTHOR_REASON`: a
 * user reading this page and then reading a run log should not be told two
 * different things about the same pipeline.
 */
export const NO_AUTHOR_REASON =
  "this pipeline has no author to read as; an administrator must assign one"

export type ReadsAs =
  /** A Dagster job has no `run_as` and no enforce call: it is not this feature's business. */
  | { kind: "not-applicable" }
  /**
   * An author is recorded. `name` is null when neither the detail read nor
   * the caller's user list can name them; `active` is null when nobody can
   * tell, which is NOT the same as `false` — only `GET /api/identity/users`
   * knows a user's status, and it is `identity:read`.
   */
  | { kind: "author"; userId: string; name: string | null; active: boolean | null }
  /** No author recorded, so under D2 the pipeline does not run. */
  | { kind: "unset" }

/**
 * What `pipeline.runAs` means for this pipeline, as one of three states.
 *
 * An author whose row the caller's user list does not carry is reported
 * with `name: null` rather than as unset: the pipeline genuinely has an
 * author (it runs), and showing "no author" there would be a lie the
 * operator would act on by assigning somebody else.
 */
export function readsAs(pipeline: PipelineDetail, users: readonly User[]): ReadsAs {
  if (pipeline.engine !== "authored") return { kind: "not-applicable" }
  const userId = pipeline.runAs
  if (!userId) return { kind: "unset" }
  const user = users.find((candidate) => candidate.id === userId)
  return {
    kind: "author",
    userId,
    // The detail read carries the name, so every viewer sees who it reads
    // as; the user list is the fallback for callers whose list DOES have
    // them, and the source of the only fact the page cannot otherwise
    // know: whether that user is still active.
    name: pipeline.runAsName ?? user?.name ?? null,
    active: user ? user.status === "active" : null,
  }
}

export type AuthorChoices = {
  /** Users that may be assigned right now, in name order. */
  options: User[]
  /**
   * Why the list is empty, when it is — `null` when there is simply
   * nobody to offer. The API's own rules, said early: a deactivated user
   * cannot read anything, and an author outside the pipeline's tenant
   * would be refused.
   */
  blocked: string | null
}

/**
 * The users a Platform Admin may pick for THIS pipeline.
 *
 * Two filters, both matching rules the API enforces on
 * `PUT .../run-as`, so the control cannot offer a choice that is refused:
 * a deactivated user (whose pipelines would not run anyway — D2) and a
 * user outside the pipeline's own tenant.
 *
 * A pipeline with no tenant has no scope to check membership against, and
 * the API refuses the assignment until one is assigned, so the list is
 * empty with that stated rather than quietly showing everyone.
 */
export function authorChoices(users: readonly User[], tenantId: string | undefined): AuthorChoices {
  if (!tenantId) {
    return {
      options: [],
      blocked: "this pipeline has no tenant yet, so there is nobody to pick from; assign it a tenant first",
    }
  }
  const options = users
    .filter((user) => user.status === "active" && user.tenants.includes(tenantId))
    .sort((a, b) => a.name.localeCompare(b.name))
  return {
    options,
    blocked: options.length === 0 ? "no active user in this pipeline's tenant can read as it" : null,
  }
}

/**
 * The line the page shows. A pipeline that cannot run says WHY rather than
 * showing an empty chip, because "no author" is the one state here an
 * operator is expected to fix.
 */
export function readsAsLine(state: ReadsAs): string | null {
  switch (state.kind) {
    case "not-applicable":
      return null
    case "author":
      if (state.name === null) return "Reads as: a user you cannot see"
      // Only SAY "deactivated" when someone can actually tell. `active:
      // null` means nobody can, and claiming a state we do not know is the
      // kind of wrong an operator acts on.
      return `Reads as: ${state.name}${state.active === false ? " (deactivated)" : ""}`
    case "unset":
      return `Reads as: not set — ${NO_AUTHOR_REASON}`
  }
}