import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import * as fs from "node:fs"
import * as path from "node:path"
import { PermissionPicker } from "./permission-picker"
import { identityService, type Permission } from "@/services"

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
  { name: "pipeline:write", area: "Pipelines", description: "Create, edit and trigger pipelines." },
  { name: "policy:read", area: "Governance", description: "View governance policies and SLAs." },
  { name: "policy:write", area: "Governance", description: "Create and edit governance policies and SLAs." },
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
    expect(screen.getByText("Governance")).toBeDefined()

    // Verify checkboxes in their groups
    expect(screen.getByRole("checkbox", { name: /catalog:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /query:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /dashboard:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /dashboard:write/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /pipeline:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /pipeline:write/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /policy:read/i })).toBeDefined()
    expect(screen.getByRole("checkbox", { name: /policy:write/i })).toBeDefined()
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

    // Data, Dashboards, Pipelines, Governance
    const allDataCheckboxes = screen.getAllByRole("checkbox", { name: /All in this area/i })
    expect(allDataCheckboxes.length).toBe(4)

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

  // WEB-8 review 1 B1: test role holding pipeline:*
  it("holders of pipeline:* see checked removable chip and disabled covered permissions", () => {
    let saved = "pipeline:*"
    const { rerender } = render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Shows chip "All pipeline: permissions"
    expect(screen.getByText(/All pipeline: permissions/i)).toBeDefined()
    // Not treated as unrecognised
    expect(screen.queryByText("(Not recognised)")).toBeNull()

    // Covered permissions pipeline:read and pipeline:write are checked and disabled
    const readCb = screen.getByRole("checkbox", { name: "pipeline:read" })
    expect(readCb.getAttribute("aria-checked")).toBe("true")
    expect(readCb.hasAttribute("disabled")).toBe(true)

    const writeCb = screen.getByRole("checkbox", { name: "pipeline:write" })
    expect(writeCb.getAttribute("aria-checked")).toBe("true")
    expect(writeCb.hasAttribute("disabled")).toBe(true)

    // Shows "covered by pipeline:*"
    expect(screen.getAllByText(/covered by pipeline:\*/i).length).toBe(2)

    // Click remove button on the chip
    const removeBtn = screen.getByRole("button", { name: /Remove pipeline:\*/i })
    fireEvent.click(removeBtn)
    expect(saved).toBe("")

    // Rerender with saved = ""
    rerender(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )
    const readCbAfter = screen.getByRole("checkbox", { name: "pipeline:read" })
    expect(readCbAfter.getAttribute("aria-checked")).toBe("false")
    expect(readCbAfter.hasAttribute("disabled")).toBe(false)
  })

  // WEB-8 review 1 B1: test role holding policy:*
  it("holders of policy:* see checked removable chip and disabled covered permissions", () => {
    let saved = "policy:*"
    render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    // Shows chip "All policy: permissions"
    expect(screen.getByText(/All policy: permissions/i)).toBeDefined()
    // Not treated as unrecognised
    expect(screen.queryByText("(Not recognised)")).toBeNull()

    // Covered permissions policy:read and policy:write are checked and disabled
    const readCb = screen.getByRole("checkbox", { name: "policy:read" })
    expect(readCb.getAttribute("aria-checked")).toBe("true")
    expect(readCb.hasAttribute("disabled")).toBe(true)

    const writeCb = screen.getByRole("checkbox", { name: "policy:write" })
    expect(writeCb.getAttribute("aria-checked")).toBe("true")
    expect(writeCb.hasAttribute("disabled")).toBe(true)

    // Shows "covered by policy:*"
    expect(screen.getAllByText(/covered by policy:\*/i).length).toBe(2)

    // Click remove button on the chip
    const removeBtn = screen.getByRole("button", { name: /Remove policy:\*/i })
    fireEvent.click(removeBtn)
    expect(saved).toBe("")
  })

  // WEB-8 review 1 B2: test failed catalog load renders ErrorState with Retry
  it("renders ErrorState with Retry on failed permissions fetch and does not render editable picker", async () => {
    spyOn(identityService, "listPermissions").mockRejectedValue(new Error("Failed to load permissions"))

    render(
      <PermissionPicker
        value="catalog:read"
        onChange={() => {}}
      />
    )

    // Wait for the error state to render
    expect(await screen.findByRole("status")).toBeDefined()
    expect(screen.getByRole("button", { name: /Retry/i })).toBeDefined()
    // Does not render editable picker
    expect(screen.queryByTestId("area-group-data")).toBeNull()
    // Does not render held tokens as Not recognised
    expect(screen.queryByText(/Not recognised/i)).toBeNull()
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

// WEB-13: a long, unbroken permission token must stay inside its chip or row.
// happy-dom computes no geometry, so the wrapping is asserted as structure;
// the measured narrow/wide fit is a manual-QA item (see the plan).
describe("PermissionPicker long identifiers (WEB-13)", () => {
  const LONG_UNKNOWN = "legacy_" + "unknown_permission_token_".repeat(4) + "final"
  const LONG_RESOURCE = "very_long_resource_name_" + "segment_".repeat(6) + "end"
  const LONG_PERMISSION = `${LONG_RESOURCE}:read`
  const description = "A long name still keeps its readable description."

  it("keeps a long unknown token whole and removable with its exact saved value", () => {
    let saved = `catalog:read, ${LONG_UNKNOWN}`
    render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={MOCK_PERMISSIONS}
      />
    )

    const chip = screen.getByText(LONG_UNKNOWN)
    expect(chip.textContent).toBe(LONG_UNKNOWN)
    expect(chip.className).toContain("break-all")
    const badge = chip.parentElement
    expect(badge?.className).toContain("max-w-full")
    expect(badge?.className).toContain("h-auto")
    expect(badge?.className).toContain("whitespace-normal")
    const remove = screen.getByRole("button", { name: `Remove ${LONG_UNKNOWN}` })
    expect(remove.className).toContain("shrink-0")

    fireEvent.click(remove)
    expect(saved).toBe("catalog:read")
  })

  it("keeps a long known permission name whole and selectable, with its description", () => {
    let saved = ""
    const permissions: Permission[] = [{ name: LONG_PERMISSION, area: "Data", description }]
    render(
      <PermissionPicker
        value={saved}
        onChange={(v) => {
          saved = v
        }}
        permissions={permissions}
      />
    )

    const labelText = screen.getByText(LONG_PERMISSION)
    expect(labelText.textContent).toBe(LONG_PERMISSION)
    expect(labelText.className).toContain("break-all")
    const label = labelText.parentElement
    expect(label?.tagName).toBe("LABEL")
    expect(label?.className).toContain("min-w-0")
    expect(label?.parentElement?.className).toContain("min-w-0")
    expect(label?.parentElement?.parentElement?.className).toContain("flex-1")
    expect(screen.getByText(description)).toBeTruthy()
    expect(screen.getByRole("checkbox", { name: LONG_PERMISSION }).className).toContain("shrink-0")

    fireEvent.click(label as HTMLElement)
    expect(saved).toBe(LONG_PERMISSION)
  })

  it("keeps a long known permission covered by a wildcard checked and disabled", () => {
    const wildcard = `${LONG_RESOURCE}:*`
    const saved = wildcard
    // The wildcard is known because the catalog names its resource; it is not
    // itself a catalog row, so only the long permission is shown as covered.
    const permissions: Permission[] = [{ name: LONG_PERMISSION, area: "Data", description }]
    render(<PermissionPicker value={saved} onChange={() => {}} permissions={permissions} />)

    expect(screen.getByText(LONG_PERMISSION).textContent).toBe(LONG_PERMISSION)
    const checkbox = screen.getByRole("checkbox", { name: LONG_PERMISSION })
    expect(checkbox.getAttribute("aria-checked")).toBe("true")
    expect(checkbox.hasAttribute("disabled")).toBe(true)
    expect(screen.getByText(`covered by ${wildcard}`)).toBeTruthy()
  })
})

/**
 * UX-1-R10: a required permission picker is a named group with a linked
 * requirement sentence. A group role has no supported aria-required, and no
 * individual checkbox is required — one token satisfies the whole group.
 */
describe("PermissionPicker required group semantics (UX-1-R10)", () => {
  it("names the group and links the requirement, without marking each checkbox", () => {
    render(
      <PermissionPicker
        id="role-perms"
        label="Permissions"
        required
        value=""
        onChange={() => {}}
        permissions={MOCK_PERMISSIONS}
      />
    )
    const group = screen.getByRole("group", { name: "Permissions" })
    expect(group.getAttribute("aria-describedby")).toBe("role-perms-required-help")
    expect(screen.getByText("Select at least one permission.")).toBeTruthy()
    // No false aria-required on the unsupported group role or on a checkbox.
    expect(group.hasAttribute("aria-required")).toBe(false)
    const checkbox = screen.getByRole("checkbox", { name: "catalog:read" })
    expect(checkbox.hasAttribute("aria-required")).toBe(false)
  })

  it("leaves an optional picker unmarked", () => {
    render(<PermissionPicker id="opt" value="" onChange={() => {}} permissions={MOCK_PERMISSIONS} />)
    const group = screen.getByRole("group", { name: "Permissions" })
    expect(group.hasAttribute("aria-describedby")).toBe(false)
  })
})
