// The re-upload switch's rules (DATA-19 D3), as unit tests: what the
// stored chain means, what the switch sends, and — the rule that matters
// most — that no other `dependsOn` entry is ever touched by it. The
// component itself is rendered in `pipeline-create-page.test.tsx`; this
// file pins the pure decision, where every branch is cheap to enumerate.
import { describe, expect, it } from "bun:test"
import {
  FILE_INGEST_JOB,
  INGEST_JOB,
  reuploadTriggerOn,
  showsReuploadSwitch,
  withReuploadTrigger,
} from "./pipeline-reupload-trigger"

describe("the re-upload switch is offered for a Bronze source only", () => {
  it("shows for bronze and for nothing else", () => {
    expect(showsReuploadSwitch("bronze")).toBe(true)
    // An uploaded file is loaded into `bronze.<table>` only
    // (`file_ingest.py`); a silver/gold source has no table to re-upload,
    // and the server refuses the upstream for those (DATA-19 D2).
    for (const zone of ["silver", "gold", "serving", "Bronze", ""]) {
      expect(showsReuploadSwitch(zone)).toBe(false)
    }
    // The form trims on submit, so a half-typed zone must not flicker the
    // switch in and out while somebody is typing.
    expect(showsReuploadSwitch(" bronze ")).toBe(true)
  })

  it("reads OFF for a stored chain that does not name the upload job", () => {
    expect(reuploadTriggerOn(["pl-upstream"], "bronze", true)).toBe(false)
    expect(reuploadTriggerOn(undefined, "bronze", true)).toBe(false)
  })

  it("reads ON for a stored chain that does name it", () => {
    expect(reuploadTriggerOn([FILE_INGEST_JOB, "pl-upstream"], "bronze", true)).toBe(true)
  })

  it("defaults ON for a pipeline that was never saved", () => {
    // The feature's point: a pipeline over an uploaded table wants the
    // re-upload to run it, so a new one starts with the switch on.
    expect(reuploadTriggerOn(undefined, "bronze", false)).toBe(true)
  })

  it("is OFF for a non-Bronze source whatever the stored chain says", () => {
    // A row stored before the server's refusal (D2) can still name the
    // upload job on a `silver` source. The switch is not shown there, so
    // it must not read as ON either.
    expect(reuploadTriggerOn([FILE_INGEST_JOB], "silver", true)).toBe(false)
    expect(reuploadTriggerOn(undefined, "silver", false)).toBe(false)
  })
})

describe("turning the switch on and off changes only that one upstream", () => {
  it("adds the upload job and keeps every other entry", () => {
    expect(withReuploadTrigger(["pl-upstream", "ingest_job"], "bronze", true)).toEqual([
      FILE_INGEST_JOB,
      "pl-upstream",
      "ingest_job",
    ])
  })

  it("removes the upload job and keeps every other entry", () => {
    expect(withReuploadTrigger([FILE_INGEST_JOB, "pl-upstream"], "bronze", false)).toEqual([
      "pl-upstream",
    ])
  })

  it("never writes the upload job twice", () => {
    expect(withReuploadTrigger([FILE_INGEST_JOB, FILE_INGEST_JOB], "bronze", true)).toEqual([
      FILE_INGEST_JOB,
    ])
  })

  it("round-trips: turning it on then off is the stored chain again", () => {
    const stored = ["pl-upstream"]
    const turnedOn = withReuploadTrigger(stored, "bronze", true)
    expect(withReuploadTrigger(turnedOn, "bronze", false)).toEqual(stored)
  })

  it("drops ingest_job for a source that is not Bronze, but keeps it on Bronze (BLD-4 review R1)", () => {
    // BLD-4 review R1 / Decision R1, asserted through the exported
    // constant so the assertion follows the value the filter compares
    // against rather than a second literal that could drift from it:
    // a Silver source with ["ingest_job", "pl-a"] saves ["pl-a"]
    expect(withReuploadTrigger([INGEST_JOB, "pl-a"], "silver", true)).toEqual(["pl-a"])
    expect(withReuploadTrigger([INGEST_JOB, "pl-a"], "silver", false)).toEqual(["pl-a"])

    // a Bronze source keeps ingest_job
    expect(withReuploadTrigger([INGEST_JOB, "pl-a"], "bronze", false)).toEqual([
      INGEST_JOB,
      "pl-a",
    ])
    expect(withReuploadTrigger([INGEST_JOB, "pl-a"], "bronze", true)).toEqual([
      FILE_INGEST_JOB,
      INGEST_JOB,
      "pl-a",
    ])
  })

  it("drops the upload job for a source that is not Bronze, which the server refuses", () => {
    // `validate_per_table_upstream` answers 400 for this chain, and the
    // user has no switch to fix it with on a `silver` source — so the
    // console must not send it back unchanged.
    expect(withReuploadTrigger([FILE_INGEST_JOB, "pl-upstream"], "silver", true)).toEqual([
      "pl-upstream",
    ])
  })

  it("sends an empty chain when there was none and the switch is off", () => {
    // An explicit empty chain CLEARS the stored one
    // (`Some(vec![])` in `UpdatePipelineInput.depends_on`), which is what
    // a user who has just switched the trigger off asked for.
    expect(withReuploadTrigger(undefined, "bronze", false)).toEqual([])
  })
})