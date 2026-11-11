import { describe, expect, it } from "bun:test"
import * as fs from "node:fs"
import * as path from "node:path"
import { canOpen, NAV_GROUPS, ROUTE_ACCESS, routeMatches } from "./nav-config"

/**
 * Normalizes a file path from `src/app` into its App Router route pattern:
 * - strips `src/app/`
 * - strips `page.tsx`
 * - removes Next.js route groups like `(admin)`, `(data)`, etc.
 */
function fileToRoutePattern(filePath: string): string {
  const relative = path.relative("src/app", filePath)
  const segments = relative
    .replace(/\\/g, "/")
    .replace(/\/page\.tsx$/, "")
    .replace(/^page\.tsx$/, "")
    .split("/")
    .filter((seg) => !seg.startsWith("(") || !seg.endsWith(")"))
    .filter(Boolean)

  return "/" + segments.join("/")
}

function findPageFiles(dir: string): string[] {
  let pages: string[] = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      pages = pages.concat(findPageFiles(full))
    } else if (entry.name === "page.tsx") {
      pages.push(full)
    }
  }
  return pages
}

const PUBLIC_PREFIXES = ["/login", "/public", "/embed", "/landing", "/render"]

function isPublicRoute(pattern: string): boolean {
  return PUBLIC_PREFIXES.some((p) => pattern === p || pattern.startsWith(`${p}/`))
}

function isRedirectPage(filePath: string): boolean {
  const content = fs.readFileSync(filePath, "utf-8")
  return content.includes("redirect(")
}

describe("ADM-13 D7: nav-config access map and drift tests", () => {
  it("globs src/app/**/page.tsx and every non-public, non-redirect page has a ROUTE_ACCESS entry", () => {
    const pages = findPageFiles("src/app")
    expect(pages.length).toBeGreaterThan(0)

    const missingEntries: { file: string; pattern: string }[] = []

    for (const pageFile of pages) {
      if (isRedirectPage(pageFile)) continue

      const routePattern = fileToRoutePattern(pageFile)
      if (isPublicRoute(routePattern)) continue

      const exists = ROUTE_ACCESS.some((entry) => entry.pattern === routePattern)
      if (!exists) {
        missingEntries.push({ file: pageFile, pattern: routePattern })
      }
    }

    expect(missingEntries).toEqual([])
  })

  it("validates that every permission in requires exists in permissions.json", () => {
    const permissionsPath = path.resolve(
      "rust/crates/lakehouse-api/tests/fixtures/permissions.json"
    )
    const raw = fs.readFileSync(permissionsPath, "utf-8")
    const catalog: { name: string }[] = JSON.parse(raw)
    const validPerms = new Set(catalog.map((p) => p.name))

    const allRequires: string[] = []

    for (const entry of ROUTE_ACCESS) {
      allRequires.push(...entry.requires)
    }
    for (const group of NAV_GROUPS) {
      for (const item of group.items) {
        allRequires.push(...item.requires)
      }
    }

    const invalid = allRequires.filter((p) => !validPerms.has(p))
    expect(invalid).toEqual([])
  })

  describe("canOpen function", () => {
    it("fails closed on unlisted routes", () => {
      const hasAll = () => true
      expect(canOpen("/some/random/unlisted/route", hasAll)).toBe(false)
      expect(canOpen("/admin/nonexistent", hasAll)).toBe(false)
    })

    it("allows any signed-in user on routes with requires: []", () => {
      const hasNone = () => false
      expect(canOpen("/account/change-password", hasNone)).toBe(true)
    })

    it("requires specific permission for gated routes", () => {
      const viewerPerms = (p: string) => p === "dashboard:read"
      expect(canOpen("/dashboards", viewerPerms)).toBe(true)
      expect(canOpen("/dashboards/browse", viewerPerms)).toBe(true)
      expect(canOpen("/dashboards/metrics", viewerPerms)).toBe(true)
      expect(canOpen("/dashboards/sales-123", viewerPerms)).toBe(true)

      expect(canOpen("/catalog", viewerPerms)).toBe(false)
      expect(canOpen("/query-studio", viewerPerms)).toBe(false)
      expect(canOpen("/admin/users", viewerPerms)).toBe(false)
    })

    it("matches dynamic route patterns correctly", () => {
      const catalogPerms = (p: string) => p === "catalog:read"
      expect(canOpen("/data/assets/customers", catalogPerms)).toBe(true)
      expect(canOpen("/lakehouse/tables/bronze/events", catalogPerms)).toBe(true)

      const pipelinePerms = (p: string) => p === "pipeline:read"
      expect(canOpen("/pipelines/orders-etl", pipelinePerms)).toBe(true)
      expect(canOpen("/pipelines/orders-etl/edit", (p: string) => p === "pipeline:write")).toBe(
        true
      )
    })

    it("gates the WEB-7 form pages with the permission their submit route needs", () => {
      const reader = (p: string) => p === "identity:read" || p === "catalog:read"
      const identityWriter = (p: string) => p === "identity:write"
      const governanceWriter = (p: string) => p === "governance:write"

      // A reader who may open the list cannot reach the form ...
      expect(canOpen("/admin/users/invite", reader)).toBe(false)
      expect(canOpen("/admin/roles/new", reader)).toBe(false)
      expect(canOpen("/admin/roles/r1/edit", reader)).toBe(false)
      expect(canOpen("/admin/tenants/new", reader)).toBe(false)
      expect(canOpen("/admin/service-identities/new", reader)).toBe(false)
      expect(canOpen("/admin/settings/tag-keys/new", reader)).toBe(false)
      expect(canOpen("/admin/settings/tag-keys/data_domain/edit", reader)).toBe(false)
      // ... and a writer can, on the permission the API route takes.
      expect(canOpen("/admin/users/invite", identityWriter)).toBe(true)
      expect(canOpen("/admin/roles/new", identityWriter)).toBe(true)
      expect(canOpen("/admin/roles/r1/edit", identityWriter)).toBe(true)
      expect(canOpen("/admin/tenants/new", identityWriter)).toBe(true)
      expect(canOpen("/admin/service-identities/new", identityWriter)).toBe(true)
      expect(canOpen("/admin/settings/tag-keys/new", governanceWriter)).toBe(true)
      expect(canOpen("/admin/settings/tag-keys/data_domain/edit", governanceWriter)).toBe(true)
    })

    it("supports any-of permission checks", () => {
      const hasAlertWrite = (p: string) => p === "alert:write"
      const hasAlertRead = (p: string) => p === "alert:read"
      const hasNeither = (p: string) => p === "query:read"

      expect(canOpen("/alerts", hasAlertWrite)).toBe(true)
      expect(canOpen("/alerts", hasAlertRead)).toBe(true)
      expect(canOpen("/alerts", hasNeither)).toBe(false)
    })
  })

  describe("routeMatches helper", () => {
    it("matches exact paths", () => {
      expect(routeMatches("/catalog", "/catalog")).toBe(true)
      expect(routeMatches("/catalog", "/catalog/")).toBe(true)
      expect(routeMatches("/catalog", "/query-studio")).toBe(false)
    })

    it("matches parameterized paths", () => {
      expect(routeMatches("/dashboards/[id]", "/dashboards/my-board")).toBe(true)
      expect(
        routeMatches(
          "/lakehouse/tables/[namespace]/[table]",
          "/lakehouse/tables/db/tbl"
        )
      ).toBe(true)
      expect(routeMatches("/dashboards/[id]", "/dashboards/my-board/extra")).toBe(
        false
      )
    })
  })
})
