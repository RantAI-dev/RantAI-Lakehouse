import { describe, expect, it } from "bun:test"
import { validateMaintenancePolicyForm } from "./lakehouse-maintenance-form"

describe("validateMaintenancePolicyForm", () => {
  it("rejects a negative snapshotsToKeep before submit", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: -1,
      orphanAgeHours: null,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(errors.snapshotsToKeep).toBeDefined()
  })

  it("rejects a zero snapshotsToKeep — the current snapshot is always kept", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: 0,
      orphanAgeHours: null,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(errors.snapshotsToKeep).toBeDefined()
  })

  it("accepts snapshotsToKeep of exactly 1", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: 1,
      orphanAgeHours: null,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(errors.snapshotsToKeep).toBeUndefined()
  })

  it("rejects a negative orphanAgeHours", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: null,
      orphanAgeHours: -5,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(errors.orphanAgeHours).toBeDefined()
  })

  it("rejects a zero orphanAgeHours — an age of 0 could delete files an in-flight write still needs", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: null,
      orphanAgeHours: 0,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(errors.orphanAgeHours).toBeDefined()
  })

  it("accepts orphanAgeHours of exactly 1", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: null,
      orphanAgeHours: 1,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(errors.orphanAgeHours).toBeUndefined()
  })

  it("rejects a schedule that is not daily, weekly, or null", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: null,
      orphanAgeHours: null,
      compactSmallFiles: false,
      schedule: "hourly",
    })
    expect(errors.schedule).toBeDefined()
  })

  it("accepts daily and weekly schedules", () => {
    expect(
      validateMaintenancePolicyForm({
        snapshotsToKeep: null,
        orphanAgeHours: null,
        compactSmallFiles: false,
        schedule: "daily",
      }).schedule
    ).toBeUndefined()
    expect(
      validateMaintenancePolicyForm({
        snapshotsToKeep: null,
        orphanAgeHours: null,
        compactSmallFiles: false,
        schedule: "weekly",
      }).schedule
    ).toBeUndefined()
  })

  it("accepts a fully-null policy (clears to defaults)", () => {
    const errors = validateMaintenancePolicyForm({
      snapshotsToKeep: null,
      orphanAgeHours: null,
      compactSmallFiles: false,
      schedule: null,
    })
    expect(Object.keys(errors)).toHaveLength(0)
  })
})
