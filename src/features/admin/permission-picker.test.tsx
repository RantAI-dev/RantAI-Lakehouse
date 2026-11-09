import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import * as fs from "node:fs"
import * as path from "node:path"
import { PermissionPicker } from "./permission-picker"
import type { Permission } from "@/services"

afterEach(() => {
  cleanup()
  mock.restore()
})

const MOCK_PERMISSIONS: Permission[] = [
  { name: "catalog:read", area: "Data", description: "Browse tables and columns in the catalog." },
  { name: "query:read", area: "Data", description: "Run SQL and read table data in Query Studio." },
  { name: "dashboard:read", area: "Dashboards", description: "View dashboards and charts." },
  { name: "dashboard:write", area: "Dashboards", description: "Create and edit dashboards and charts." },
  { name: "pipeline:read", area: "Pipelines", description: "View pipelines and their runs." },
]

describe("PermissionPicker (WEB-8 T3 / D7)", () => {
  it("checkboxes are grouped by area", () => {
    render(
      <PermissionPicker
        value="catalog:read"
        onChange={() => {}}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Verify area headings
    expect(screen.getByText("Data")).toBeDefined()
    expect(screen.getByText("Dashboards")).toBeDefined()
    expect(screen.getByText("Pipelines")).toBeDefined()

    // Verify checkboxes in their groups
    expect(screen.getByRole("checkbox", { name: /catalog:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /query:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /dashboard:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /dashboard:write/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /pipeline:read/i })).toBeDefined()
  })

  it("each area has an 'All in this area' checkbox that toggles all in that area", () => {
    let saved = "catalog:read"
    const { rerender } = render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Data has catalog:read and query:read. Currently only catalog:read is checked.
    const allDataCheckboxes = screen.getAllByRole("checkbox", { name: /All in this area/i })
    expect(allDataCheckboxes.length).toBe(3) // Data, Dashboards, Pipelines

    // Clicking "All in this area" for Data checks all in Data
    fireEvent.click(allDataCheckboxes[0])
    expect(saved).toBe("catalog:read, query:read")

    // Rerender with all checked in Data
    rerender(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Clicking "All in this area" again unchecks all in Data
    const updatedAllDataCheckboxes = screen.getAllByRole("checkbox", { name: /All in this area/i })
    fireEvent.click(updatedAllDataCheckboxes[0])
    expect(saved).toBe("")
  })

  it("an unknown token shows as 'Not recognised' and can be removed", () => {
    let saved = "catalog:read, notebook:run"
    render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Shows "notebook:run" and "Not recognised"
    expect(screen.getByText("notebook:run")).toBeDefined()
    expect(screen.getByText("(Not recognised)")).toBeDefined()

    // Click remove button on the unknown token chip
    const removeBtn = screen.getByRole("button", { name: /Remove notebook:run/i })
    fireEvent.click(removeBtn)

    // Token is removed, keeping catalog:read
    expect(saved).toBe("catalog:read")
  })

  it("the saved payload is the comma string the API takes today", () => {
    let saved = ""
    const { rerender } = render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Click catalog:read
    const catalogCheck = screen.getByRole("checkbox", { name: /catalog:read/i })
    fireEvent.click(catalogCheck)
    expect(saved).toBe("catalog:read")

    // If starting with catalog:read and clicking dashboard:write
    saved = "catalog:read"
    rerender(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )
    const dashWriteCheck = screen.getByRole("checkbox", { name: /dashboard:write/i })
    fireEvent.click(dashWriteCheck)
    expect(saved).toBe("catalog:read, dashboard:write")
  })

  it("holders of *:* see one 'Full access (*:*)' checkbox", () => {
    let saved = "*:*"
    render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Sees "Full access (*:*)" checkbox
    expect(screen.getByText(/Full access/i)).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /Full access/i })).toBeDefined()

    // Unchecking *:* removes it
    fireEvent.click(screen.getByRole("checkbox", { name: /Full access/i }))
    expect(saved).toBe("")
  })

  it("there is no query:run anywhere in src/", () => {
    const srcDir = path.resolve(__dirname, "../../")
    const forbidden = "query:" + "run"
    function scanDir(dir: string) {
      const entries = fs.readdirSync(dir, { withFileTypes: true })
      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name)
        if (entry.isDirectory()) {
          scanDir(fullPath)
        } else if (/\.(tsx?|jsx?|json|css|html)$/.test(entry.name)) {
          if (fullPath === __filename) continue
          const content = fs.readFileSync(fullPath, "utf8")
          expect(content.includes(forbidden)).toBe(false)
        }
      }
    }
    scanDir(srcDir)
  })
})
