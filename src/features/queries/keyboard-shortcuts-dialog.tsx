"use client"

import * as React from "react"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { EDITOR_SHORTCUTS } from "@/lib/editor-shortcuts"

/**
 * Query Studio's own keys, which the page binds rather than the editor:
 * listed apart from `EDITOR_SHORTCUTS`, whose test checks every entry against
 * the editor's keymap (`UX-9`, the command menu).
 */
const PAGE_SHORTCUTS = [
  { action: "Open the command menu", label: "⌘/Ctrl+Shift+P" },
  // UX-9 review: Firefox on Windows and Linux keeps Ctrl+Shift+P for itself.
  { action: "Open the command menu (any browser)", label: "F1" },
] as const

/**
 * A dialog listing the keyboard shortcuts the SQL editor binds. The list
 * comes from `EDITOR_SHORTCUTS` — nothing is hard-coded here — so a change
 * to the list is reflected automatically. `UX-9`: it opens from the ⋯ menu
 * and the command menu, so the page holds whether it is open.
 */
export function KeyboardShortcutsDialog({
  open,
  onOpenChange,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>
            Keys the SQL editor binds. Run and selection shortcuts work inside
            the editor and from the toolbar.
          </DialogDescription>
        </DialogHeader>
        {/* A definition list gives each action and its key accessible
            structure. The key is styled like a keyboard key so it stands out
            visually without needing a separate column. */}
        <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-2 text-sm">
          {EDITOR_SHORTCUTS.map((shortcut) => (
            <React.Fragment key={shortcut.key}>
              <dt className="text-foreground">{shortcut.action}</dt>
              <dd className="text-right">
                <kbd className="rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">
                  {shortcut.label}
                </kbd>
              </dd>
            </React.Fragment>
          ))}
          {PAGE_SHORTCUTS.map((shortcut) => (
            <React.Fragment key={shortcut.label}>
              <dt className="text-foreground">{shortcut.action}</dt>
              <dd className="text-right">
                <kbd className="rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">
                  {shortcut.label}
                </kbd>
              </dd>
            </React.Fragment>
          ))}
        </dl>
      </DialogContent>
    </Dialog>
  )
}
