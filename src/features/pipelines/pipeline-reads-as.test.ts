import { describe, expect, test } from "bun:test"
import type { User } from "@/services/contracts/identity"
import type { PipelineDetail } from "@/services/contracts/pipelines"
import { authorChoices, readsAs, readsAsLine, NO_AUTHOR_REASON } from "./pipeline-reads-as"

function user(overrides: Partial<User> & Pick<User, "id" | "name">): User {
  return {
    email: `${overrides.name.toLowerCase()}@meridian.example`,
    status: "active",
    roles: ["Analyst"],
    tenants: ["meridian-group"],
    lastActivity: null,
    ...overrides,
  }
}

function pipeline(overrides: Partial<PipelineDetail> = {}): PipelineDetail {
  return {
    id: "pl-orders",
    name: "Orders",
    kind: "batch",
    status: "ready",
    owner: "Current user",
    source: "bronze.orders",
    target: "silver.orders_clean",
    schedule: "0 3 * * *",
    lastRunAt: null,
    slaOk: null,
    freshnessLagSeconds: null,
    engine: "authored",
    description: null,
    graph: null,
    config: [],
    definition: null,
    tenantId: "meridian-group",
    ...overrides,
  }
}

const rina = user({ id: "u-rina", name: "Rina Wijaya", tenants: ["meridian-group"] })
const bayu = user({ id: "u-bayu", name: "Bayu Pratama", tenants: ["other-tenant"] })

describe("readsAs", () => {
  test("an authored pipeline with no author is the state that stops it running", () => {
    expect(readsAs(pipeline(), [rina])).toEqual({ kind: "unset" })
    expect(readsAsLine(readsAs(pipeline(), [rina]))).toContain(NO_AUTHOR_REASON)
  })

  test("an authored pipeline names its author", () => {
    expect(readsAs(pipeline({ runAs: "u-rina" }), [rina])).toEqual({
      kind: "author",
      userId: "u-rina",
      name: "Rina Wijaya",
      active: true,
    })
    expect(readsAsLine(readsAs(pipeline({ runAs: "u-rina" }), [rina]))).toBe("Reads as: Rina Wijaya")
  })

  test("a deactivated author is still an author, and says so", () => {
    const inactive = { ...rina, status: "inactive" as const }
    const state = readsAs(pipeline({ runAs: "u-rina" }), [inactive])
    expect(state.kind).toBe("author")
    expect(readsAsLine(state)).toBe("Reads as: Rina Wijaya (deactivated)")
  })

  test("an author outside the caller's user list is never reported as unset", () => {
    // The pipeline really does have an author and really does run; saying
    // "no author" would send an operator to assign somebody else.
    const state = readsAs(pipeline({ runAs: "u-someone" }), [rina])
    expect(state).toEqual({ kind: "author", userId: "u-someone", name: null, active: null })
    expect(readsAsLine(state)).toBe("Reads as: a user you cannot see")
  })

  test("the detail read's own name is used, so a viewer who cannot list users still sees it", () => {
    // `GET /api/identity/users` is `identity:read`; a Data Engineer is the
    // pipelines' main audience and does not hold it.
    const state = readsAs(pipeline({ runAs: "u-rina", runAsName: "Rina Wijaya" }), [])
    expect(state).toEqual({ kind: "author", userId: "u-rina", name: "Rina Wijaya", active: null })
    expect(readsAsLine(state)).toBe("Reads as: Rina Wijaya")
  })

  test("an author nobody can place is not labelled deactivated", () => {
    // `active: null` means "unknown", and claiming otherwise would put a
    // false statement in front of an operator deciding what to do.
    expect(readsAsLine(readsAs(pipeline({ runAs: "u-x", runAsName: "Someone" }), []))).toBe(
      "Reads as: Someone"
    )
  })

  test("a Dagster job is not this feature's business", () => {
    expect(readsAs(pipeline({ engine: "dagster", runAs: "u-rina" }), [rina])).toEqual({
      kind: "not-applicable",
    })
    expect(readsAsLine(readsAs(pipeline({ engine: "dagster" }), [rina]))).toBeNull()
  })
})

describe("authorChoices", () => {
  test("offers the pipeline's own tenant, active only, in name order", () => {
    const choices = authorChoices(
      [
        bayu,
        rina,
        { ...rina, id: "u-adi", name: "Adi Pratama" },
        // A distinct name, so a filter that let a deactivated user through
        // would be visible rather than hidden among the expected entries.
        { ...rina, id: "u-off", name: "Zoe Offline", status: "inactive" },
      ],
      "meridian-group"
    )
    expect(choices.blocked).toBeNull()
    expect(choices.options.map((candidate) => candidate.name)).toEqual(["Adi Pratama", "Rina Wijaya"])
  })

  test("a pipeline with no tenant offers nobody and says why", () => {
    const choices = authorChoices([rina], undefined)
    expect(choices.options).toEqual([])
    expect(choices.blocked).toContain("no tenant")
  })

  test("an empty tenant says so rather than looking like a failed load", () => {
    const choices = authorChoices([], "meridian-group")
    expect(choices.options).toEqual([])
    expect(choices.blocked).toBe("no active user in this pipeline's tenant can read as it")
  })
})