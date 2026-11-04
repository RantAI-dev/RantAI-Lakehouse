// The three helpers every download shares: the name the server gave the
// file, the hand-off to the browser, and the sentence of a refusal.
//
// They live in `src/lib` because a service client needs them too, and a
// client may not import from `src/features`.
import { describe, expect, it } from "bun:test"
import { fileNameOf, refusalOf, saveBlob } from "./browser-file"

describe("the name of a downloaded file", () => {
  it("is the name the server sent in the Content-Disposition header", () => {
    expect(fileNameOf('attachment; filename="Visits.csv"')).toBe("Visits.csv")
  })

  it("is the fallback when the header names no file", () => {
    expect(fileNameOf(null)).toBe("table.csv")
    expect(fileNameOf("attachment")).toBe("table.csv")
  })

  it("is the fallback the caller named, when it names one", () => {
    expect(fileNameOf(null, "query.csv")).toBe("query.csv")
  })
})

describe("the sentence of a refused download", () => {
  it("is the server's own sentence, without the error code in front of it", async () => {
    const res = new Response(
      JSON.stringify({ error: "permission_denied: Downloads are switched off for your role." }),
      { status: 403 },
    )
    expect(await refusalOf(res, "The download failed.")).toBe(
      "Downloads are switched off for your role.",
    )
  })

  it("is the server's own sentence when it carries no code", async () => {
    const res = new Response(
      JSON.stringify({ error: "This result is larger than 5 GB. Add a filter or download it as CSV." }),
      { status: 422 },
    )
    expect(await refusalOf(res, "The download failed.")).toBe(
      "This result is larger than 5 GB. Add a filter or download it as CSV.",
    )
  })

  it("is the fallback when the body is not JSON", async () => {
    const res = new Response("<html>bad gateway</html>", { status: 503 })
    expect(await refusalOf(res, "The download failed.")).toBe("The download failed.")
  })

  it("is the fallback when the body holds no sentence", async () => {
    const res = new Response(JSON.stringify({}), { status: 503 })
    expect(await refusalOf(res, "The download failed.")).toBe("The download failed.")
  })
})

describe("saving a file", () => {
  it("hands the file to the browser under the name it was given, and takes the link away again", () => {
    const clicked: string[] = []
    const original = HTMLAnchorElement.prototype.click
    HTMLAnchorElement.prototype.click = function recordClick(this: HTMLAnchorElement) {
      clicked.push(this.download)
    }
    try {
      saveBlob(new Blob(["a,b\r\n"]), "q-1.csv")
    } finally {
      HTMLAnchorElement.prototype.click = original
    }
    expect(clicked).toEqual(["q-1.csv"])
    expect(document.querySelector("a[download]")).toBeNull()
  })
})
