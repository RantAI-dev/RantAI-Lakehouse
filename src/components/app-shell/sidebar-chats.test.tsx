/**
 * `WEB-18`: on a conversation (`/copilot`) the sidebar lists the chats
 * instead of the menu; "Back to menu" brings the menu back without leaving
 * the conversation, and the menu's "AI chats" row returns to the list.
 *
 * `next/navigation` is stubbed at the module boundary with an address the
 * test sets, and the API with a fetch stub that answers the session check
 * and the chat list (filtered by `q` as the server does).
 */
let currentPathname = "/copilot"
const router = {
  push: mock<(href: string) => void>(() => {}),
  replace: mock<(href: string) => void>(() => {}),
  refresh: () => {},
  back: () => {},
  forward: () => {},
  prefetch: () => {},
}
mock.module("next/navigation", () => ({
  usePathname: () => currentPathname,
  useSearchParams: () => new URLSearchParams("id=c1"),
  useRouter: () => router,
}))
mock.module("next/image", () => ({ default: () => null }))

import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import { CopilotProvider } from "@/features/copilot/use-copilot"
import { SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AppSidebar } from "./app-sidebar"

const json = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })

const today = new Date().toISOString()
const CHATS = [
  { id: "c1", title: "Quarterly revenue", mode: "ask", updatedAt: today, preview: "Revenue grew 4%" },
  { id: "c2", title: "Orders pipeline", mode: "build", updatedAt: today, preview: "Built the pipeline" },
  { id: "c3", title: "Churn by region", mode: "ask", updatedAt: today, preview: "Churn is flat" },
]

const originalFetch = globalThis.fetch
let listCalls: string[] = []

function stubApi(permissions: string[]) {
  listCalls = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Ana", email: "ana@example.invalid", roles: ["R"], permissions, tenants: [] })
    }
    if (url.startsWith("/api/ai/sessions") && !url.includes("id=")) {
      listCalls.push(url)
      const q = new URL(url, "http://x").searchParams.get("q")?.toLowerCase() ?? ""
      const sessions = CHATS.filter(
        (s) => !q || s.title.toLowerCase().includes(q) || s.preview.toLowerCase().includes(q),
      )
      return json({ sessions, hasMore: false, nextOffset: null, counts: {} })
    }
    return json({})
  }) as typeof fetch
}

beforeEach(() => {
  router.push.mockClear()
  router.replace.mockClear()
})

afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

function renderSidebar({ open = true }: { open?: boolean } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <AuthProvider>
          <CopilotProvider>
            <SidebarProvider open={open} onOpenChange={() => {}}>
              <SidebarTrigger />
              <AppSidebar />
            </SidebarProvider>
          </CopilotProvider>
        </AuthProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  )
}

/**
 * The chat rows' titles, in order. A plain selector rather than a role query:
 * role queries over the whole sidebar are slow under happy-dom, and a
 * `waitFor` re-runs them every 50 ms.
 */
const chatTitles = () =>
  Array.from(document.querySelectorAll<HTMLAnchorElement>('a[href^="/copilot?id="]')).map((a) => a.textContent)

/** The view on screen: the one not marked inert. */
const visible = (el: Element) => el.closest("[inert]") === null

describe("the sidebar's chat list on a conversation (WEB-18)", () => {
  it("lists the chats on /copilot, with the four actions on top, instead of the menu", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask", "catalog:read", "settings:write"])
    renderSidebar()

    const back = await screen.findByRole("button", { name: "Back to menu" })
    expect(visible(back)).toBe(true)
    expect(screen.getByRole("button", { name: "New chat" })).toBeTruthy()
    expect(screen.getByRole("searchbox", { name: "Search chats" })).toBeTruthy()
    expect(screen.getByRole("link", { name: "AI settings" }).getAttribute("href")).toBe(
      "/admin/settings?section=assistant",
    )
    const row = await screen.findByRole("link", { name: "Quarterly revenue" })
    expect(row.getAttribute("href")).toBe("/copilot?id=c1")
    expect(screen.getByText("Today")).toBeTruthy()
    // The menu is not what is shown.
    expect(screen.queryAllByRole("link", { name: "Catalog" }).filter(visible)).toHaveLength(0)
  })

  it("shows the menu, with no AI chats row, anywhere else", async () => {
    currentPathname = "/catalog"
    stubApi(["assistant:ask", "catalog:read"])
    renderSidebar()

    expect(await screen.findByRole("link", { name: "Catalog" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Back to menu" })).toBeNull()
    expect(screen.queryByRole("button", { name: "AI chats" })).toBeNull()
    // The Copilot's own recent list (`limit=20`) loads on every page; the chat list does not.
    expect(listCalls.filter((u) => u.includes("limit=30"))).toHaveLength(0)
  })

  it("Back to menu shows the menu and keeps the conversation; AI chats brings the list back", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask", "catalog:read"])
    renderSidebar()

    fireEvent.click(await screen.findByRole("button", { name: "Back to menu" }))
    const aiChats = await screen.findByRole("button", { name: "AI chats" })
    expect(visible(aiChats)).toBe(true)
    expect(screen.getAllByRole("link", { name: "Catalog" }).some(visible)).toBe(true)
    // The page is not left: no navigation happened.
    expect(router.push).not.toHaveBeenCalled()
    expect(router.replace).not.toHaveBeenCalled()

    fireEvent.click(aiChats)
    const back = await screen.findByRole("button", { name: "Back to menu" })
    expect(visible(back)).toBe(true)
  })

  it("Search filters the list on titles and message text", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask"])
    renderSidebar()

    await waitFor(() => expect(chatTitles()).toEqual(["Quarterly revenue", "Orders pipeline", "Churn by region"]))
    const search = screen.getByRole("searchbox", { name: "Search chats" })
    fireEvent.change(search, { target: { value: "churn" } })
    await waitFor(() => expect(chatTitles()).toEqual(["Churn by region"]), { timeout: 3000 })
    expect(listCalls.some((u) => u.includes("q=churn"))).toBe(true)

    // A match in the last message, not the title.
    fireEvent.change(search, { target: { value: "grew" } })
    await waitFor(() => expect(chatTitles()).toEqual(["Quarterly revenue"]), { timeout: 3000 })

    // Esc clears it.
    fireEvent.keyDown(search, { key: "Escape" })
    expect((search as HTMLInputElement).value).toBe("")
    await waitFor(() => expect(chatTitles()).toHaveLength(3), { timeout: 3000 })
  })

  it('"/" focuses the search while the list is shown, but not while typing elsewhere', async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask"])
    renderSidebar()

    const search = await screen.findByRole("searchbox", { name: "Search chats" })
    const composer = document.createElement("textarea")
    document.body.appendChild(composer)
    composer.focus()
    fireEvent.keyDown(composer, { key: "/" })
    expect(document.activeElement).toBe(composer)

    composer.blur()
    act(() => {
      fireEvent.keyDown(document.body, { key: "/" })
    })
    expect(document.activeElement).toBe(search)
    composer.remove()
  })

  it("hides AI settings from someone who cannot change them", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask"])
    renderSidebar()

    await screen.findByRole("button", { name: "Back to menu" })
    expect(screen.queryByRole("link", { name: "AI settings" })).toBeNull()
  })

  it("New chat goes to Home, where a conversation starts", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask"])
    renderSidebar()

    fireEvent.click(await screen.findByRole("button", { name: "New chat" }))
    expect(router.push).toHaveBeenCalledWith("/")
  })
})

// T3: the two views slide sideways inside the sidebar, both at once; collapsed, the actions stay as
// icons; on a phone the sheet holds the same list.
describe("the slide between the sidebar's two views (WEB-18 T3)", () => {
  const view = (name: "menu" | "chats") => document.querySelector(`[data-sidebar-view="${name}"]`) as HTMLElement

  it("moves the menu out to the left and the chats in from the right, and back, at the sidebar's width", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask", "catalog:read"])
    renderSidebar()

    await screen.findByRole("button", { name: "Back to menu" })
    expect(view("menu").className).toContain("-translate-x-full")
    expect(view("menu").hasAttribute("inert")).toBe(true)
    expect(view("chats").className).not.toContain("translate-x-full")
    // One move: the same duration and curve for both, and none under reduced motion.
    for (const pane of [view("menu"), view("chats")]) {
      expect(pane.className).toContain("transition-transform")
      expect(pane.className).toContain("duration-[240ms]")
      expect(pane.className).toContain("ease-[cubic-bezier(0.23,1,0.32,1)]")
      expect(pane.className).toContain("motion-reduce:transition-none")
    }

    fireEvent.click(screen.getByRole("button", { name: "Back to menu" }))
    await waitFor(() => expect(view("chats").className).toContain("translate-x-full"))
    expect(view("chats").hasAttribute("inert")).toBe(true)
    expect(view("menu").className).not.toContain("-translate-x-full")
    expect(view("menu").hasAttribute("inert")).toBe(false)
    // The focus follows to the control that leads back.
    await waitFor(() => expect(document.activeElement?.textContent).toContain("AI chats"))
  })

  it("collapsed: the four actions stay as icons with tooltips and the chats fold away", async () => {
    currentPathname = "/copilot"
    stubApi(["assistant:ask", "settings:write"])
    renderSidebar({ open: false })

    const back = await screen.findByRole("button", { name: "Back to menu" })
    expect(back.closest("[data-collapsible='icon']")).toBeTruthy()
    // Collapsed, Search is a button that opens the sidebar on the field.
    expect(screen.getByRole("button", { name: "Search chats" })).toBeTruthy()
    expect(screen.getByRole("searchbox", { name: "Search chats" }).getAttribute("tabindex")).toBe("-1")
    await waitFor(() => expect(chatTitles()).toHaveLength(3))
    const list = screen.getByRole("navigation", { name: "Chats" })
    expect(list.hasAttribute("inert")).toBe(true)
  })

  it("on a phone the sidebar's sheet shows the chat list, with Back to menu", async () => {
    const width = window.innerWidth
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 })
    try {
      currentPathname = "/copilot"
      stubApi(["assistant:ask", "catalog:read"])
      renderSidebar()
      fireEvent.click(await screen.findByRole("button", { name: "Toggle Sidebar" }))
      const sheet = await screen.findByRole("dialog")
      expect(sheet.getAttribute("data-mobile")).toBe("true")
      expect(within(sheet).getByRole("button", { name: "Back to menu" })).toBeTruthy()
      await waitFor(() => expect(chatTitles()).toHaveLength(3))
    } finally {
      Object.defineProperty(window, "innerWidth", { configurable: true, value: width })
    }
  })
})
