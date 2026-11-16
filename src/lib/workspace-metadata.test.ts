/**
 * Guard tests for the browser tab (`ADM-2`, plan decisions 3, 4 and 7).
 *
 * The tab title and the tab icon belong to the administrator who set them:
 * the console reads the public branding answer and shows the workspace's own
 * name and icon, and falls back to the product's own when the route cannot
 * answer. These tests exist so a later pull request cannot quietly put the
 * product's fixed name and icon back — the owner asked for this guard
 * through the Auditor (2026-11-16), because nobody tells the author of the
 * next redesign that the name is the workspace's now.
 *
 * The last three tests read the source instead of calling a function: the
 * root layout and the route pages are Next.js modules whose metadata only a
 * running Next server exercises, so the guard reads the files a later pull
 * request would rewrite.
 */
import { afterEach, describe, expect, it, mock } from "bun:test"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

import {
  readWorkspaceBranding,
  workspaceIconLinks,
  workspaceMetadata,
} from "./workspace-metadata"

/** The `src/` directory, so the source guards read the tree they belong to. */
const SRC_DIR = fileURLToPath(new URL("..", import.meta.url))

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

/** The public branding answer, as the route gives it. */
function brandingAnswer(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * A request that never answers until its signal aborts.
 *
 * A request that carries no signal at all never settles: a reader that does
 * not time its request would hold the page's render forever, and the test
 * would time out instead of passing quietly.
 */
function neverAnswers(init?: RequestInit): Promise<Response> {
  return new Promise<Response>((_resolve, reject) => {
    const signal = init?.signal
    if (signal === undefined || signal === null) return
    if (signal.aborted) {
      reject(new Error("the request was aborted"))
      return
    }
    signal.addEventListener("abort", () => reject(new Error("the request timed out")))
  })
}

/** Answers every request with `respond`. */
function stubFetch(
  respond: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>,
): void {
  globalThis.fetch = mock(respond) as unknown as typeof fetch
}

/** The pages that carry a title of their own, and the part that is theirs. */
const TITLED_PAGES = [
  { file: "app/(admin)/account/change-password/page.tsx", title: "Change password" },
  { file: "app/(overview)/alerts/page.tsx", title: "Alerts & Digests" },
  { file: "app/embed/dashboard/[token]/page.tsx", title: "Embedded dashboard" },
  { file: "app/embed/signed/[jwt]/page.tsx", title: "Embedded dashboard" },
  { file: "app/login/page.tsx", title: "Sign in" },
  { file: "app/public/dashboard/[token]/page.tsx", title: "Shared dashboard" },
  { file: "app/render/chart/[token]/page.tsx", title: "Chart image" },
  { file: "app/render/dashboard/[token]/page.tsx", title: "Dashboard PDF" },
]

describe("the browser tab title", () => {
  it("the browser tab shows the workspace name from settings, not a fixed product name", async () => {
    stubFetch(async () =>
      brandingAnswer({ workspaceName: "Acme Data", logo: null, logoDark: null, icon: null }),
    )

    const metadata = workspaceMetadata(await readWorkspaceBranding({ timeoutMs: 100 }))

    expect(metadata.title).toEqual({ default: "Acme Data", template: "%s · Acme Data" })
    expect(metadata.appleWebApp).toEqual({ title: "Acme Data" })
    expect(JSON.stringify(metadata.title)).not.toContain("Rantai Lake")
  })

  it("the browser tab falls back to the product's own name when the branding API is unreachable", async () => {
    stubFetch(async () => {
      throw new Error("connection refused")
    })

    const metadata = workspaceMetadata(await readWorkspaceBranding({ timeoutMs: 100 }))

    expect(metadata.title).toEqual({ default: "Rantai Lake", template: "%s · Rantai Lake" })
  })

  it("the browser tab falls back to the product's own name when the branding API answers 500", async () => {
    stubFetch(async () => new Response("boom", { status: 500 }))

    const metadata = workspaceMetadata(await readWorkspaceBranding({ timeoutMs: 100 }))

    expect(metadata.title).toEqual({ default: "Rantai Lake", template: "%s · Rantai Lake" })
  })

  it("the browser tab falls back to the product's own name when the branding answer is not the branding shape", async () => {
    const answers = [
      brandingAnswer({ workspaceName: 42 }),
      brandingAnswer({ workspaceName: "Acme Data", icon: { type: 7, version: "v1" } }),
      new Response("not json at all", { status: 200 }),
    ]

    for (const answer of answers) {
      stubFetch(async () => answer)

      const metadata = workspaceMetadata(await readWorkspaceBranding({ timeoutMs: 100 }))

      expect(metadata.title).toEqual({ default: "Rantai Lake", template: "%s · Rantai Lake" })
    }
  })

  it("the browser tab falls back to the product's own name when the branding request times out", async () => {
    stubFetch(async (_input, init) => neverAnswers(init))

    const startedAt = Date.now()
    const metadata = workspaceMetadata(await readWorkspaceBranding({ timeoutMs: 50 }))
    const elapsed = Date.now() - startedAt

    expect(metadata.title).toEqual({ default: "Rantai Lake", template: "%s · Rantai Lake" })
    // The reader gave up on the timeout it was given, not on the production
    // one: a tab title must not hold a page's render for three seconds.
    expect(elapsed).toBeLessThan(1_500)
  })
})

describe("the browser tab icon", () => {
  it("the browser tab icon points at the workspace's own icon when one is set", async () => {
    stubFetch(async () =>
      brandingAnswer({
        workspaceName: "Acme Data",
        logo: null,
        logoDark: null,
        icon: { type: "image/png", version: "v1" },
      }),
    )

    const branding = await readWorkspaceBranding({ timeoutMs: 100 })
    const links = workspaceIconLinks(branding)

    expect(links).toEqual([
      { rel: "icon", href: "/api/branding/images/icon?v=v1", type: "image/png" },
      { rel: "apple-touch-icon", href: "/api/branding/images/icon?v=v1" },
    ])
    const hrefs = links.map((link) => link.href)
    expect(hrefs).not.toContain("/icon-light-32.png")
    expect(hrefs).not.toContain("/icon-dark-32.png")
  })

  it("the browser tab icon stays the product's own when the workspace set none", () => {
    expect(workspaceIconLinks(null)).toEqual([
      {
        rel: "icon",
        href: "/icon-light-32.png",
        type: "image/png",
        media: "(prefers-color-scheme: light)",
      },
      {
        rel: "icon",
        href: "/icon-dark-32.png",
        type: "image/png",
        media: "(prefers-color-scheme: dark)",
      },
      { rel: "apple-touch-icon", href: "/apple-touch-icon.png" },
    ])
  })
})

describe("the source guards", () => {
  it("the root layout reads the workspace name instead of a fixed product name", () => {
    const source = readFileSync(join(SRC_DIR, "app/layout.tsx"), "utf8")

    expect(source).toContain("export async function generateMetadata")
    expect(source).not.toContain('title: "Rantai Lake"')
  })

  it("the pages with their own title keep only their own part", () => {
    const withTheProductName = TITLED_PAGES.filter((page) =>
      readFileSync(join(SRC_DIR, page.file), "utf8").includes("Rantai Lake"),
    ).map((page) => page.file)

    expect(withTheProductName).toEqual([])
  })

  it("the pages with their own title still say what the page is", () => {
    const withoutTheirOwnTitle = TITLED_PAGES.filter(
      (page) => !readFileSync(join(SRC_DIR, page.file), "utf8").includes(`title: "${page.title}"`),
    ).map((page) => page.file)

    expect(withoutTheirOwnTitle).toEqual([])
  })
})
