"use client"

import * as React from "react"
import { Keyboard } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { EDITOR_SHORTCUTS } from "@/lib/editor-shortcuts"

/**
 * A button that opens a dialog listing the keyboard shortcuts the SQL
 * editor binds. The list comes from `EDITOR_SHORTCUTS` — nothing is
 * hard-coded here — so a change to the list is reflected automatically.
 */
export function KeyboardShortcutsDialog() {
  return (
    <Dialog>
      <DialogTrigger render={<Button size="sm" variant="outline" />}>
        <Keyboard className="size-4" aria-hidden />
        Keyboard shortcuts
      </DialogTrigger>
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
        </dl>
      </DialogContent>
    </Dialog>
  )
}
