"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { ChevronLeft, MoreHorizontal, Plus, Search, Settings, X } from "lucide-react"

import {
  SidebarGroup,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
  SIDEBAR_FADE,
  useSidebar,
} from "@/components/ui/sidebar"
import { Spinner } from "@/components/ui/spinner"
import { useAuth } from "@/features/auth/auth-provider"
import { SessionActionsMenu } from "@/features/copilot/session-parts"
import { useChatList, useFollowRecentSessions } from "@/features/copilot/use-chat-list"
import { useCopilot } from "@/features/copilot/use-copilot"
import { useDebouncedCallback } from "@/hooks/use-debounced-callback"
import { useScrollFade } from "@/hooks/use-scroll-fade"
import { groupSessions } from "@/lib/copilot-sessions"
import { cn } from "@/lib/utils"
import { ChatListSkeleton } from "./chat-list-skeleton"
import { SIDEBAR_GROUP_CLASS, SIDEBAR_MENU_CLASS, navRowClass } from "./sidebar-classes"

/** Where "AI settings" goes: the Assistant section of Settings (`ADM-9`). */
export const AI_SETTINGS_HREF = "/admin/settings?section=assistant"

/** A conversation's address; `SessionUrlSync` on `/copilot` opens it. */
export function chatHref(id: string): string {
  return `/copilot?id=${encodeURIComponent(id)}`
}

/** The key that focuses the chat search, as on the app's other lists. */
const SEARCH_KEY = "/"

/** Typing into a field, or a key with a modifier: "/" is a character there, not a shortcut. */
function isTypingTarget(e: KeyboardEvent): boolean {
  if (e.metaKey || e.ctrlKey || e.altKey) return true
  const t = e.target as HTMLElement | null
  if (!t) return false
  return t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName)
}

/**
 * The sidebar's second view, on a conversation (`/copilot`, `WEB-18`): the
 * chat history lives here, as in a chat app, instead of on a History page
 * (owner 2026-11-14, "Pas sudah di sesi conversation, sidebar itulah yang jadi
 * list historynya, jadi kek Claude").
 *
 * Top to bottom: "Back to menu" (slides the main menu back, without leaving
 * the conversation), "New chat" (Home, where a conversation starts, with its
 * composer focused), "Search" (filters this list; "/" focuses it, Esc
 * clears), "AI settings" (the Assistant section of Settings, for whoever may
 * change it), then the chats, newest first, grouped by day, a page at a time.
 *
 * Collapsed to icons, the four actions stay as icons with tooltips and the
 * chats fold away; Search opens the sidebar and focuses the field.
 */
export function SidebarChats({
  shown,
  onBack,
}: {
  /** The view is the one on screen: "/" is listened for, and the list loads. */
  readonly shown: boolean
  readonly onBack: () => void
}) {
  const router = useRouter()
  const c = useCopilot()
  const { hasPermission } = useAuth()
  const { state, isMobile, setOpen, setOpenMobile } = useSidebar()
  const iconMode = state === "collapsed" && !isMobile
  const canSetUpAi = hasPermission("settings:write")

  const [search, setSearch] = React.useState("")
  const [q, setQ] = React.useState("")
  const pushSearch = useDebouncedCallback((next: string) => setQ(next), 300)
  const inputRef = React.useRef<HTMLInputElement>(null)

  // The list loads the first time it is shown, and stays loaded after.
  const [everShown, setEverShown] = React.useState(shown)
  React.useEffect(() => {
    if (shown) setEverShown(true)
  }, [shown])
  const list = useChatList(q, everShown)
  useFollowRecentSessions(c.sessions)
  const groups = React.useMemo(() => groupSessions(list.sessions), [list.sessions])

  const focusSearch = React.useCallback(() => {
    if (iconMode) {
      // Collapsed there is no field to type in: open the sidebar, then focus
      // it once the field is laid out at its full width.
      setOpen(true)
      window.requestAnimationFrame(() => inputRef.current?.focus())
      return
    }
    inputRef.current?.focus()
  }, [iconMode, setOpen])

  React.useEffect(() => {
    if (!shown || isMobile) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== SEARCH_KEY || isTypingTarget(e)) return
      e.preventDefault()
      focusSearch()
    }
    document.addEventListener("keydown", onKey)
    return () => document.removeEventListener("keydown", onKey)
  }, [shown, isMobile, focusSearch])

  const clearSearch = () => {
    setSearch("")
    setQ("")
  }

  const startNew = () => {
    c.newChat()
    if (isMobile) setOpenMobile(false)
    router.push("/")
  }

  // Load the next page when the end of the list scrolls into view.
  const fadeRef = useScrollFade<HTMLDivElement>()
  const [scroller, setScroller] = React.useState<HTMLDivElement | null>(null)
  const scrollRef = React.useCallback(
    (el: HTMLDivElement | null) => {
      fadeRef(el)
      setScroller(el)
    },
    [fadeRef],
  )
  const sentinel = React.useRef<HTMLDivElement>(null)
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = list
  React.useEffect(() => {
    const el = sentinel.current
    if (!el || !scroller || !hasNextPage || typeof IntersectionObserver === "undefined") return
    const io = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting && !isFetchingNextPage) void fetchNextPage()
      },
      { root: scroller, rootMargin: "120px" },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [scroller, hasNextPage, isFetchingNextPage, fetchNextPage])

  return (
    <>
      <SidebarGroup className={cn(SIDEBAR_GROUP_CLASS, "shrink-0")}>
        <SidebarMenu className={SIDEBAR_MENU_CLASS}>
          <SidebarMenuItem>
            <SidebarMenuButton
              data-view-focus="chats"
              tooltip="Back to menu"
              className={navRowClass(false)}
              onClick={onBack}
            >
              <ChevronLeft className="size-4 shrink-0" />
              <span className={cn("truncate leading-5", SIDEBAR_FADE)}>Back to menu</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton tooltip="New chat" className={navRowClass(false)} onClick={startNew}>
              <Plus className="size-4 shrink-0" />
              <span className={cn("truncate leading-5", SIDEBAR_FADE)}>New chat</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            {/* One element in both states, like every row here: expanded it is the field, with
                the icon where a row's icon sits; collapsed the field fades and the row is a
                button that opens the sidebar on it. */}
            <SidebarMenuButton
              tooltip="Search chats"
              render={<div />}
              role={iconMode ? "button" : undefined}
              tabIndex={iconMode ? 0 : undefined}
              aria-label={iconMode ? "Search chats" : undefined}
              className={cn(
                navRowClass(false),
                "cursor-text focus-within:bg-sidebar-accent focus-within:text-sidebar-accent-foreground",
                iconMode && "cursor-pointer",
              )}
              onMouseDown={(e: React.MouseEvent) => {
                // A click anywhere on the row lands in the field, not on the row.
                if (!iconMode && e.target !== inputRef.current) {
                  e.preventDefault()
                  inputRef.current?.focus()
                }
              }}
              onClick={() => {
                if (iconMode) focusSearch()
              }}
              onKeyDown={(e: React.KeyboardEvent) => {
                if (iconMode && (e.key === "Enter" || e.key === " ")) {
                  e.preventDefault()
                  focusSearch()
                }
              }}
            >
              <Search className="size-4 shrink-0" />
              <input
                ref={inputRef}
                type="search"
                value={search}
                tabIndex={iconMode ? -1 : undefined}
                onChange={(e) => {
                  setSearch(e.target.value)
                  pushSearch(e.target.value.trim())
                }}
                onKeyDown={(e) => {
                  if (e.key !== "Escape") return
                  e.preventDefault()
                  if (search) clearSearch()
                  else e.currentTarget.blur()
                }}
                placeholder="Search"
                aria-label="Search chats"
                className={cn(
                  "h-5 min-w-0 flex-1 bg-transparent text-sm leading-5 text-sidebar-accent-foreground outline-none placeholder:text-sidebar-foreground/70 [&::-webkit-search-cancel-button]:appearance-none",
                  SIDEBAR_FADE,
                )}
              />
              {search ? (
                <button
                  type="button"
                  onClick={clearSearch}
                  aria-label="Clear search"
                  className={cn(
                    "grid size-5 shrink-0 place-items-center rounded text-sidebar-foreground/60 transition-colors duration-150 hover:text-sidebar-accent-foreground",
                    SIDEBAR_FADE,
                  )}
                >
                  <X className="size-3.5" />
                </button>
              ) : null}
            </SidebarMenuButton>
          </SidebarMenuItem>
          {canSetUpAi ? (
            <SidebarMenuItem>
              <SidebarMenuButton
                tooltip="AI settings"
                className={navRowClass(false)}
                render={
                  <Link href={AI_SETTINGS_HREF} onClick={() => isMobile && setOpenMobile(false)}>
                    <Settings className="size-4 shrink-0" />
                    <span className={cn("truncate leading-5", SIDEBAR_FADE)}>AI settings</span>
                  </Link>
                }
              />
            </SidebarMenuItem>
          ) : null}
        </SidebarMenu>
      </SidebarGroup>

      {/* The chats. Collapsed they fold away (there is no room for a title) and take no focus;
          opening the sidebar shows them again. */}
      <div
        ref={scrollRef}
        inert={iconMode}
        aria-label="Chats"
        role="navigation"
        className={cn("scroll-fade no-scrollbar min-h-0 flex-1 overflow-y-auto px-2 pb-2", SIDEBAR_FADE)}
      >
        {list.isPending ? (
          <ChatListSkeleton />
        ) : list.isError ? (
          <div className="flex flex-col items-start gap-1 px-2 py-2 text-xs text-sidebar-foreground/70">
            <span>Couldn&apos;t load your chats.</span>
            <button
              type="button"
              onClick={() => void list.refetch()}
              className="font-medium text-sidebar-accent-foreground underline-offset-4 hover:underline"
            >
              Try again
            </button>
          </div>
        ) : groups.length === 0 ? (
          <p className="px-2 py-2 text-xs text-sidebar-foreground/70">
            {q ? <>No chats match &ldquo;{q}&rdquo;.</> : "No chats yet. Ask something on Home to start one."}
          </p>
        ) : (
          <>
            {groups.map(({ group, items }) => (
              <section key={group} aria-label={group} className="pt-2 first:pt-1">
                <h3 className="truncate px-2 pb-1 text-xs font-medium text-sidebar-foreground/60">{group}</h3>
                <SidebarMenu className={SIDEBAR_MENU_CLASS}>
                  {items.map((s) => {
                    const active = s.id === c.sessionId
                    const title = s.title || "Untitled conversation"
                    return (
                      <SidebarMenuItem key={s.id}>
                        <SidebarMenuButton
                          isActive={active}
                          className={navRowClass(active)}
                          render={
                            <Link
                              href={chatHref(s.id)}
                              title={title}
                              aria-current={active ? "page" : undefined}
                              onClick={() => isMobile && setOpenMobile(false)}
                            >
                              <span className="truncate leading-5">{title}</span>
                            </Link>
                          }
                        />
                        <SessionActionsMenu
                          session={{ ...s, title }}
                          onRename={(next) => c.renameSession(s.id, next)}
                          onDelete={() => c.removeSession(s.id)}
                          trigger={
                            <SidebarMenuAction
                              showOnHover
                              className="right-1 text-sidebar-foreground/60"
                            >
                              <MoreHorizontal />
                            </SidebarMenuAction>
                          }
                        />
                      </SidebarMenuItem>
                    )
                  })}
                </SidebarMenu>
              </section>
            ))}
            <div ref={sentinel} className="flex h-8 items-center justify-center text-sidebar-foreground/60">
              {isFetchingNextPage ? <Spinner /> : null}
            </div>
          </>
        )}
      </div>
    </>
  )
}
