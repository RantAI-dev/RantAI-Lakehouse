import { describe, expect, it } from "bun:test"
import {
  NO_ACCESS,
  ServiceError,
  deniedDetail,
  deniedFor,
  isDeniedText,
  readDenied,
} from "./errors"

describe("readDenied", () => {
  it("names the missing permission when the API sends its id after the code", () => {
    expect(readDenied("permission_denied: connector:manage")).toEqual({
      message: "You don't have access. This needs the connector:manage permission.",
      detail: "This needs the connector:manage permission.",
    })
  })

  it("lists the permissions when the API names several, any one of which would do", () => {
    expect(readDenied("permission_denied: alert:read, alert:write").message).toBe(
      "You don't have access. This needs one of these permissions: alert:read, alert:write."
    )
  })

  it("shows a sentence the server wrote as it is, without the code", () => {
    expect(readDenied("permission_denied: Downloads are switched off for your role.")).toEqual({
      message: "Downloads are switched off for your role.",
      detail: "Downloads are switched off for your role.",
    })
  })

  it("gives a fragment the server wrote as the reason", () => {
    expect(readDenied("permission_denied: you cannot grant access to yourself")).toEqual({
      message: "You don't have access. Reason given: you cannot grant access to yourself.",
      detail: "Reason given: you cannot grant access to yourself.",
    })
  })

  it("says only the fixed sentence when the API names nothing a person can read", () => {
    for (const raw of ["permission_denied", "permission_denied:", "embedding_disabled", "unauthorized", "", undefined, null, 42]) {
      expect(readDenied(raw)).toEqual({ message: NO_ACCESS, detail: "" })
    }
  })

  it("leaves words that carry no code as they are", () => {
    expect(readDenied("You do not have permission to do this.")).toEqual({
      message: "You do not have permission to do this.",
      detail: "You do not have permission to do this.",
    })
  })
})

describe("deniedFor", () => {
  it("turns a 401 or a 403 into permission_denied with the status, the sentence and the detail", () => {
    const forbidden = deniedFor(403, { error: "permission_denied: pipeline:read" })
    expect(forbidden).toBeInstanceOf(ServiceError)
    expect(forbidden?.code).toBe("permission_denied")
    expect(forbidden?.status).toBe(403)
    expect(forbidden?.message).toBe("You don't have access. This needs the pipeline:read permission.")
    expect(forbidden?.detail).toBe("This needs the pipeline:read permission.")

    const unauthorized = deniedFor(401, { error: "unauthorized" })
    expect(unauthorized?.status).toBe(401)
    expect(unauthorized?.message).toBe(NO_ACCESS)

    for (const body of [null, "text"]) {
      const plain = deniedFor(403, body)
      expect(plain?.message).toBe(NO_ACCESS)
      expect(plain?.detail).toBe("")
    }
  })

  it("answers null for every status that is not a refusal", () => {
    for (const status of [200, 400, 404, 409, 410, 422, 429, 500, 503]) {
      expect(deniedFor(status, { error: "permission_denied: x:y" })).toBeNull()
    }
  })
})

describe("deniedDetail", () => {
  it("reads the detail from the coded message of an error a client has not mapped yet", () => {
    expect(deniedDetail(new ServiceError("unavailable", "permission_denied: catalog:read"))).toBe(
      "This needs the catalog:read permission."
    )
    expect(deniedDetail(new ServiceError("permission_denied", "Failed (403)"))).toBe("")
    expect(deniedDetail(deniedFor(403, { error: "permission_denied: x:y" })!)).toBe(
      "This needs the x:y permission."
    )
  })
})

describe("isDeniedText", () => {
  it("tells the API's code-first text from every other text", () => {
    expect(isDeniedText("permission_denied")).toBe(true)
    expect(isDeniedText(" permission_denied: a:b")).toBe(true)
    expect(isDeniedText("permissions: nothing")).toBe(false)
    expect(isDeniedText("permission_denied_extra: x")).toBe(false)
    expect(isDeniedText("Chart not found.")).toBe(false)
    expect(isDeniedText("")).toBe(false)
  })
})
