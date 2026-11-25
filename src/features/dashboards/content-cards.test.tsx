import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, spyOn } from "bun:test"

import { dashboardService } from "@/services"
import { ReportingProvider } from "./reporting-context"
import { FRAME_REFUSED, WebCard } from "./content-cards"

// happy-dom would otherwise try to load the page a frame points at; the test reads the frame only.
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

afterEach(() => {
  cleanup()
})

const URL_OK = "https://status.example/page"
const reporting = {
  timeZone: "UTC", weekStart: "monday" as const, numberLocale: "en-US" as const,
  companyPalette: [], fonts: [], contentSites: ["https://status.example"],
}

function card() {
  return (
    <ReportingProvider reporting={reporting}>
      <WebCard def={{ url: URL_OK } as never} title="Status" />
    </ReportingProvider>
  )
}

describe("WebCard (BI-72)", () => {
  it("says so once, with an Open in a new tab button, when the site refuses framing", async () => {
    const spy = spyOn(dashboardService, "frameCheck").mockResolvedValue(false)
    const { container } = render(card())
    await waitFor(() => expect(screen.getByText(FRAME_REFUSED)).toBeTruthy())
    expect(container.querySelector("iframe")).toBeNull()
    const link = screen.getByText("Open in a new tab").closest("a")
    expect(link?.getAttribute("href")).toBe(URL_OK)
    expect(link?.getAttribute("rel")).toBe("noopener noreferrer")
    expect(spy).toHaveBeenCalledTimes(1)
    spy.mockRestore()
  })

  it("shows the frame when the site allows it, and when the server could not tell", async () => {
    for (const answer of [true, null]) {
      const spy = spyOn(dashboardService, "frameCheck").mockResolvedValue(answer)
      const { container } = render(card())
      await waitFor(() => expect(container.querySelector("iframe")).not.toBeNull())
      expect(screen.queryByText(FRAME_REFUSED)).toBeNull()
      cleanup()
      spy.mockRestore()
    }
  })

  it("shows a placeholder, not the frame, while it asks", () => {
    const spy = spyOn(dashboardService, "frameCheck").mockImplementation(() => new Promise(() => {}))
    const { container } = render(card())
    expect(container.querySelector("iframe")).toBeNull()
    expect(container.querySelector(".animate-pulse")).not.toBeNull()
    spy.mockRestore()
  })

  it("never asks about a site that is no longer allowed", () => {
    const spy = spyOn(dashboardService, "frameCheck").mockResolvedValue(true)
    render(
      <ReportingProvider reporting={{ ...reporting, contentSites: [] }}>
        <WebCard def={{ url: URL_OK } as never} title="Status" />
      </ReportingProvider>,
    )
    expect(screen.getByText("This site is no longer allowed")).toBeTruthy()
    expect(spy).not.toHaveBeenCalled()
    spy.mockRestore()
  })
})
