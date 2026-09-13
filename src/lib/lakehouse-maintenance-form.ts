/**
 * Pure client-side validation for the maintenance-policy form on the
 * table-detail page (WS2 §4). Mirrors
 * `validate_maintenance_policy_body` in
 * `rust/crates/lakehouse-api/src/routes/lakehouse.rs` field for field and
 * message for message, so a bad input fails here before the round trip the
 * server would otherwise reject with the same 400.
 */

export type MaintenancePolicyFormValues = {
  snapshotsToKeep: number | null
  orphanAgeHours: number | null
  compactSmallFiles: boolean
  schedule: string | null
}

export type MaintenancePolicyFormErrors = {
  snapshotsToKeep?: string
  orphanAgeHours?: string
  schedule?: string
}

/**
 * Validates `values` against the same bounds the route's
 * `validate_maintenance_policy_body` enforces: `snapshotsToKeep` and
 * `orphanAgeHours` must each be at least 1 when set, and `schedule` must be
 * `"daily"`, `"weekly"`, or `null`. Returns an empty object when `values`
 * would be accepted.
 */
export function validateMaintenancePolicyForm(
  values: MaintenancePolicyFormValues
): MaintenancePolicyFormErrors {
  const errors: MaintenancePolicyFormErrors = {}

  if (values.snapshotsToKeep !== null && values.snapshotsToKeep < 1) {
    errors.snapshotsToKeep =
      "snapshotsToKeep must be at least 1: the current snapshot is always kept"
  }

  if (values.orphanAgeHours !== null && values.orphanAgeHours < 1) {
    errors.orphanAgeHours =
      "orphanAgeHours must be at least 1: an age of 0 could delete files an in-flight write still needs"
  }

  if (values.schedule !== null && values.schedule !== "daily" && values.schedule !== "weekly") {
    errors.schedule = 'schedule must be "daily", "weekly", or null'
  }

  return errors
}
