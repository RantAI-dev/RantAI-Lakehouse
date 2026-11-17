"use client"

import * as React from "react"
import { Pencil, Plus, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { notifyInfo } from "@/lib/notify"
import { MAX_QUERY_TABS } from "@/lib/query-draft"
import { cn } from "@/lib/utils"
import type { useQueryStudio } from "./use-query-studio"

/**
 * The query tabs inside the SQL editor: which query is open, what it is
 * called, and the way to open another one or close this one.
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

  function handleAdd() {
    if (addTab()) return
    notifyInfo(
      `At most ${MAX_QUERY_TABS} query tabs are kept`,
      "Close a tab before opening another one."
    )
  }

  return (
    <div role="group" aria-label="Query tabs" className="flex flex-wrap items-center gap-1">
      {tabs.map((tab) => {
        const active = tab.id === activeTabId
        const renaming = renamingId === tab.id
        return (
          <div
            key={tab.id}
            className={cn(
              "flex items-center gap-0.5 rounded-lg border px-1",
              active ? "border-border bg-muted" : "border-transparent"
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
                className="h-6 w-32"
              />
            ) : (
              <>
                <Button
                  size="sm"
                  variant={active ? "secondary" : "ghost"}
                  aria-current={active ? "true" : undefined}
                  onClick={() => setActiveTab(tab.id)}
                >
                  {tab.name}
                </Button>
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label={`Rename ${tab.name}`}
                  onClick={() => startRename(tab.id, tab.name)}
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
            >
              <X className="size-3" aria-hidden />
            </Button>
          </div>
        )
      })}
      <Button size="icon-xs" variant="outline" aria-label="Add query tab" onClick={handleAdd}>
        <Plus className="size-3" aria-hidden />
      </Button>
    </div>
  )
}
