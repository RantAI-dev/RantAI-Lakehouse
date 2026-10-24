// The connector-load switch's rules (BLD-7 decisions 4 and 5), as unit
// tests: what the stored chain means, what the switch sends, and the rule
// that matters most -- that no other `dependsOn` entry is ever touched by
// it. The component wiring is rendered in `pipeline-create-page.test.tsx`.
import { describe, expect, it } from "bun:test"
import {
  connectorLoadTriggerOn,
  withConnectorLoadTrigger,
} from "./pipeline-connector-trigger"
import { FILE_INGEST_JOB, INGEST_JOB } from "./pipeline-reupload-trigger"

describe("the connector-load switch is offered for a Bronze source with a connector only", () => {
  it("reads OFF without a connector and for a non-Bronze source", () => {
    expect(connectorLoadTriggerOn(undefined, "bronze", false, false)).toBe(false)
    expect(connectorLoadTriggerOn([INGEST_JOB], "bronze", false, true)).toBe(false)
    expect(connectorLoadTriggerOn([INGEST_JOB], "silver", true, true)).toBe(false)
    expect(connectorLoadTriggerOn([INGEST_JOB], "gold", true, false)).toBe(false)
    // The form trims on submit, so a half-typed zone must not flicker the
    // switch in and out while somebody is typing.
    expect(connectorLoadTriggerOn(undefined, " bronze ", true, false)).toBe(true)
  })

  it("defaults ON for a pipeline that was never saved", () => {
    // The entry point is the connector page's Create pipeline: a pipeline
    // made there runs after the table it reads has been loaded.
    expect(connectorLoadTriggerOn(undefined, "bronze", true, false)).toBe(true)
  })

  it("reads the stored chain for a saved pipeline", () => {
    expect(connectorLoadTriggerOn(undefined, "bronze", true, true)).toBe(false)
    expect(connectorLoadTriggerOn(["pl-upstream"], "bronze", true, true)).toBe(false)
    expect(connectorLoadTriggerOn([INGEST_JOB, "pl-upstream"], "bronze", true, true)).toBe(true)
  })
})

describe("turning the connector-load switch changes only that one upstream", () => {
  it("adds the ingest job and keeps every other entry", () => {
    expect(
      withConnectorLoadTrigger([FILE_INGEST_JOB, "pl-upstream"], "bronze", true, true)
    ).toEqual([INGEST_JOB, FILE_INGEST_JOB, "pl-upstream"])
  })

  it("removes the ingest job when the switch is off, keeping the rest", () => {
    expect(
      withConnectorLoadTrigger([INGEST_JOB, FILE_INGEST_JOB, "pl-upstream"], "bronze", true, false)
    ).toEqual([FILE_INGEST_JOB, "pl-upstream"])
  })

  it("never duplicates an ingest job already in the chain", () => {
    expect(
      withConnectorLoadTrigger([INGEST_JOB, INGEST_JOB], "bronze", true, true)
    ).toEqual([INGEST_JOB])
  })

  it("drops the ingest job for a connector on a non-Bronze source", () => {
    // A row stored before the API's rule: no control is shown for the
    // upstream there, and the server refuses it, so saving must not send
    // it back (BLD-4 `validate_per_table_upstream`).
    expect(
      withConnectorLoadTrigger([INGEST_JOB, "pl-upstream"], "silver", true, true)
    ).toEqual(["pl-upstream"])
  })

  it("returns a connector-less chain untouched, whatever it holds", () => {
    // The switch is not shown without a connector, so the control must not
    // edit that chain -- whoever added `ingest_job` there owns it.
    expect(
      withConnectorLoadTrigger([INGEST_JOB, "pl-upstream"], "bronze", false, false)
    ).toEqual([INGEST_JOB, "pl-upstream"])
    expect(withConnectorLoadTrigger(undefined, "silver", false, false)).toEqual([])
  })
})
