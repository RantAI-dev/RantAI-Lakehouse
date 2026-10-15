/**
 * `/api/governance/tag-keys` (`DATA-12` part 2): the governed tag keys an
 * admin defines and the values each may take. A table is then given
 * `key = value` pairs from them (`AssetSummary.governedTags`); the free
 * one-word tags (`AssetSummary.tags`) are a separate list and are not
 * touched by any of this.
 *
 * Field names mirror the API's serde names. Reading needs `catalog:read`;
 * defining, changing and removing a key needs `governance:write`.
 */
export type TagKey = {
  /** 1-64 characters of `^[a-z0-9][a-z0-9_-]*$`; never renamed. */
  key: string
  /** At most 50 values, each shaped like the key. */
  allowedValues: string[]
  /** Display name of whoever defined the key. */
  createdBy: string
  /** RFC 3339. */
  createdAt: string
  /** RFC 3339: when the key or its values last changed. */
  updatedAt: string
}

/**
 * Every refusal rejects with a `ServiceError` whose message is the API's own
 * sentence: a value outside the pattern, a repeated value, a key that
 * already exists, and — on `update` and `remove` — a value or key some tables
 * still use, with their number.
 */
export interface TagKeysService {
  /** Every defined key, ordered by key. */
  list(signal?: AbortSignal): Promise<TagKey[]>
  /** Defines a key; needs `governance:write`. */
  create(key: string, allowedValues: string[], signal?: AbortSignal): Promise<TagKey>
  /**
   * Replaces the values a key allows; needs `governance:write`. A value a
   * table still has cannot be removed.
   */
  update(key: string, allowedValues: string[], signal?: AbortSignal): Promise<TagKey>
  /** Removes a key no table has a value for; needs `governance:write`. */
  remove(key: string, signal?: AbortSignal): Promise<void>
}
