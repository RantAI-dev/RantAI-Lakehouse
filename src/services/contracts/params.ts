import type { ParamDef } from "@/lib/params"

/**
 * Parameters (BI-11, with AI-6): named values a viewer changes on a dashboard.
 * Mirrors `rust/crates/lakehouse-bi/src/params.rs` and the routes in
 * `rust/crates/lakehouse-api/src/routes/params.rs`. Reading needs
 * `dashboard:read`; defining, changing and deleting need `dashboard:write`.
 */

/** What a person defines; `name` and `type` cannot be changed afterwards. */
export type ParamInput = Omit<ParamDef, "id" | "createdBy" | "updatedAt" | "label"> & { label?: string }

export type ParamService = {
  list(signal?: AbortSignal): Promise<ParamDef[]>
  /** A refusal (a name in use, a default outside the range) arrives as `invalid_request` with the server's sentence. */
  create(input: ParamInput, signal?: AbortSignal): Promise<ParamDef>
  update(id: string, input: ParamInput, signal?: AbortSignal): Promise<ParamDef>
  /** Refused with the server's sentence, naming what uses it, while a formula, chart or dashboard does. */
  remove(id: string, signal?: AbortSignal): Promise<void>
}
