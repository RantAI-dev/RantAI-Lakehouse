"use client"

import * as React from "react"
import { FileCode2, Pencil, Plus, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { MAX_QUERY_TABS } from "@/lib/query-draft"
import { cn } from "@/lib/utils"
import type { useQueryStudio } from "./use-query-studio"

/**
 * The query tabs above the editor: which query is open, what it is called,
 * and the way to open another one or close this one. `UX-9`: one row that
 * scrolls sideways, a tab's rename and close buttons quiet until the tab is
 * open, hovered or focused; a double-click on the name renames it too.
 *
 * The tabs live in this browser and not on the server, so the bar shows
 * what the browser holds. It refuses to open more tabs than the browser
 * keeps and says so: a button that does nothing without a word is a dead
 * end.
 */
export function QueryTabBar({ studio }: { readonly studio: ReturnType<typeof useQueryStudio> }) {
  const { tabs, activeTabId, addTab, renameTab, closeTab, setActiveTab } = studio
  const [renamingId, setRenamingId] = React.useState<string | null>(null)
  const [draftName, setDraftName] = React.useState("")
  const inputRef = React.useRef<HTMLInputElement | null>(null)
  // Escape cancels the rename; the blur that follows it must not commit.
  const cancelledRef = React.useRef(false)

  React.useEffect(() => {
    if (renamingId === null) return
    const input = inputRef.current
    if (!input) return
    input.focus()
    input.select()
  }, [renamingId])

  function startRename(id: string, name: string) {
    cancelledRef.current = false
    setDraftName(name)
    setRenamingId(id)
  }

  function commitRename(id: string) {
    if (cancelledRef.current) {
      cancelledRef.current = false
      return
    }
    // A name that is empty is refused: the tab keeps the one it had.
    renameTab(id, draftName)
    setRenamingId(null)
  }

  function cancelRename() {
    cancelledRef.current = true
    setRenamingId(null)
  }

  // QS-11 (LH-QS-1-03), D9: at the limit the "+" is disabled and the reason
  // is on the page, beside the button — a toast vanishes and leaves a dead
  // control with no explanation.
  const full = tabs.length >= MAX_QUERY_TABS

  function handleAdd() {
    addTab()
  }

  const quiet =
    "opacity-0 transition-opacity duration-150 ease-ui group-hover/tab:opacity-70 group-focus-within/tab:opacity-100 focus-visible:opacity-100"

  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <div
        role="group"
        aria-label="Query tabs"
        className="flex min-w-0 items-center gap-0.5 overflow-x-auto [scrollbar-width:none]"
      >
        {tabs.map((tab) => {
          const active = tab.id === activeTabId
          const renaming = renamingId === tab.id
          return (
            <div
              key={tab.id}
              className={cn(
                "group/tab flex h-8 shrink-0 items-center gap-0.5 rounded-lg pr-0.5 pl-1 transition-colors duration-150 ease-ui",
                active ? "bg-muted dark:bg-input/40" : "hover:bg-muted/60"
              )}
            >
              {renaming ? (
                <Input
                  ref={inputRef}
                  aria-label={`Rename ${tab.name}`}
                  value={draftName}
                  onChange={(event) => setDraftName(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault()
                      commitRename(tab.id)
                    } else if (event.key === "Escape") {
                      event.preventDefault()
                      cancelRename()
                    }
                  }}
                  onBlur={() => commitRename(tab.id)}
                  className="h-6 w-36 text-[13px]"
                />
              ) : (
                <>
                  <button
                    type="button"
                    aria-current={active ? "true" : undefined}
                    onClick={() => setActiveTab(tab.id)}
                    onDoubleClick={() => startRename(tab.id, tab.name)}
                    className={cn(
                      "flex h-7 items-center gap-1.5 rounded-md px-1.5 text-[13px] outline-none focus-visible:ring-3 focus-visible:ring-ring/50",
                      active ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground"
                    )}
                  >
                    <FileCode2 className="size-3.5 opacity-70" aria-hidden />
                    <span className="max-w-40 truncate">{tab.name}</span>
                  </button>
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    aria-label={`Rename ${tab.name}`}
                    onClick={() => startRename(tab.id, tab.name)}
                    className={active ? "opacity-70 hover:opacity-100" : quiet}
                  >
                    <Pencil className="size-3" aria-hidden />
                  </Button>
                </>
              )}
              <Button
                size="icon-xs"
                variant="ghost"
                aria-label={`Close ${tab.name}`}
                onClick={() => closeTab(tab.id)}
                className={active ? "opacity-70 hover:opacity-100" : quiet}
              >
                <X className="size-3" aria-hidden />
              </Button>
            </div>
          )
        })}
      </div>
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label="Add query tab"
        title="New query tab"
        aria-describedby={full ? "query-tab-limit" : undefined}
        disabled={full}
        onClick={handleAdd}
        className="shrink-0"
      >
        <Plus className="size-4" aria-hidden />
      </Button>
      {full ? (
        <span id="query-tab-limit" className="truncate text-xs text-muted-foreground">
          At most {MAX_QUERY_TABS} query tabs are kept. Close a tab to open another.
        </span>
      ) : null}
    </div>
  )
}
