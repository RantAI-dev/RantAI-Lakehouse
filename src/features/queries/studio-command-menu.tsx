"use client"

import * as React from "react"

import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command"
import type { StudioAction } from "./studio-actions"

const GROUPS: readonly StudioAction["group"][] = ["Query", "Edit", "Data", "View"]

/**
 * Whether a key press opens the command menu: ⌘⇧P (Ctrl+Shift+P), or F1.
 * `UX-9` review: Firefox on Windows and Linux keeps Ctrl+Shift+P for a
 * private window, so F1 (VS Code's other key for the same menu) is a way in
 * that every browser hands to the page.
 */
export function isCommandMenuKey(event: KeyboardEvent): boolean {
  if (event.key === "F1" && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
    return true
  }
  return (event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === "p"
}

/**
 * The studio's command menu (`UX-9`, mockup B): ⌘⇧P or F1 lists every editor
 * action, searchable, with its key. It is an extra way in: every action in
 * it is also on the toolbar, in its menus or in the side panel.
 *
 * A disabled action stays in the list, greyed, with its reason, so a person
 * looking for it learns why it does nothing instead of not finding it.
 */
export function StudioCommandMenu({
  open,
  onOpenChange,
  actions,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly actions: readonly StudioAction[]
}) {
  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!isCommandMenuKey(event)) return
      event.preventDefault()
      onOpenChange(!open)
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [open, onOpenChange])

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Query Studio commands"
      description="Every action on the query in the editor"
    >
      <Command>
        <CommandInput placeholder="Type a command" />
        <CommandList>
          <CommandEmpty>No command matches.</CommandEmpty>
          {GROUPS.map((group) => {
            const items = actions.filter((action) => action.group === group)
            if (items.length === 0) return null
            return (
              <CommandGroup key={group} heading={group}>
                {items.map((action) => {
                  const Icon = action.icon
                  return (
                    <CommandItem
                      key={action.id}
                      value={action.label}
                      disabled={action.disabled}
                      title={action.disabled ? action.reason : undefined}
                      onSelect={() => {
                        onOpenChange(false)
                        action.run()
                      }}
                    >
                      <Icon aria-hidden />
                      {action.label}
                      {action.hint ? <CommandShortcut>{action.hint}</CommandShortcut> : null}
                    </CommandItem>
                  )
                })}
              </CommandGroup>
            )
          })}
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
