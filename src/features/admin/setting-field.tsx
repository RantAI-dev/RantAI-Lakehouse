"use client"

import * as React from "react"
import { Label } from "@/components/ui/label"
import type { SettingSource } from "@/services"

/** What the person is told about a setting the server's environment owns. */
export const SET_BY_SERVER = "Set by the server"

/**
 * One setting of a form: its label, and the reason it cannot be changed
 * when the server's environment owns it (plan D1).
 *
 * Shared by the settings forms that write one of the deployment's own
 * settings (email, sign-in, assistant), so the hint is worded the same way
 * everywhere.
 */
export function SettingField({
  label,
  source,
  id,
  children,
}: {
  label: string
  source: SettingSource
  id: string
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {source === "server" ? (
        <p className="text-xs text-muted-foreground">{SET_BY_SERVER}</p>
      ) : null}
    </div>
  )
}
