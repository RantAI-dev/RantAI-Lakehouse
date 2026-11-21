"use client"

import * as React from "react"
import { Bot, ChevronDown, Ellipsis, FileCode2, PanelRight, Play, Save, Square } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { TabsList, TabsTrigger } from "@/components/ui/tabs"
import { formatDuration } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { StudioAction } from "./studio-actions"
import type { useQueryStudio } from "./use-query-studio"

type Studio = ReturnType<typeof useQueryStudio>

/**
 * SQL | Ask: how the question is written (`UX-9`: Ask stays a tab, owner
 * 2026-11-21). The panels are the page's, under the same `Tabs` root.
 */
export function ModeTabs() {
  return (
    <TabsList aria-label="Write in" className="h-8 shrink-0 dark:bg-input/30">
      <TabsTrigger value="sql" className="px-2.5 text-[13px]">
        <FileCode2 className="size-3.5" aria-hidden />
        SQL
      </TabsTrigger>
      <TabsTrigger value="nl" className="px-2.5 text-[13px]">
        <Bot className="size-3.5" aria-hidden />
        Ask
      </TabsTrigger>
    </TabsList>
  )
}

/** A keyboard key, quiet, inside a button or a menu. */
export function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[18px] items-center rounded px-1 font-sans text-[10px] font-medium tabular-nums",
        className
      )}
      {...props}
    />
  )
}

/** The elapsed time the way the Stop button reads it: "2.4 s", "184 ms". */
function shortDuration(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`
}

/**
 * Run, and while a run (or a download, which is a query on the engine too)
 * is going the same button is Stop, with the time beside it: one control in
 * one place (`UX-9`, mockup A). The time is also a `timer` for a reader who
 * cannot see the button change.
 */
export function RunButton({ studio }: { readonly studio: Studio }) {
  const { running, download, engine, runnable, statements, elapsedMs } = studio
  if (running || download.busy !== null) {
    const title = running
      ? engine === "trino"
        ? "Stops the page waiting; the query keeps running on Trino."
        : "Ends the query on the engine and stops the page waiting."
      : "Stops the download and ends the query on the engine."
    return (
      <>
        <Button
          variant="outline"
          onClick={studio.stopRun}
          title={title}
          aria-label="Stop"
          className="min-w-[92px] tabular-nums"
        >
          <Square className="size-3.5 fill-current" aria-hidden />
          Stop
          {running ? <span aria-hidden>· {shortDuration(elapsedMs ?? 0)}</span> : null}
        </Button>
        {running ? (
          <span role="timer" aria-label="Elapsed time" className="sr-only">
            {formatDuration(elapsedMs ?? 0)}
          </span>
        ) : null}
      </>
    )
  }
  return (
    <Button onClick={() => void studio.runQuery()} disabled={!runnable} className="min-w-[92px]">
      <Play className="size-3.5 fill-current" aria-hidden />
      {statements.length > 1 ? "Run all" : "Run"}
      <Kbd className="ml-0.5 hidden bg-white/15 text-white/85 sm:inline-flex" aria-hidden>
        ⌘↵
      </Kbd>
    </Button>
  )
}

function ActionItem({ action }: { readonly action: StudioAction }) {
  const Icon = action.icon
  return (
    <DropdownMenuItem
      disabled={action.disabled}
      title={action.disabled ? action.reason : undefined}
      onClick={action.run}
    >
      <Icon aria-hidden />
      <span className="flex-1">{action.label}</span>
      {action.hint ? (
        <span className="text-xs text-muted-foreground tabular-nums">{action.hint}</span>
      ) : null}
    </DropdownMenuItem>
  )
}

/**
 * Save, the save people use daily, with its menu beside it: "Save as new"
 * (when Save changes a saved query) and "Save as SQL source" (`UX-9`).
 * Without either, Save is a button alone.
 */
export function SaveButton({
  save,
  more,
}: {
  readonly save: StudioAction
  readonly more: readonly StudioAction[]
}) {
  return (
    <div className="inline-flex shrink-0 items-center">
      <Button
        variant="ghost"
        onClick={save.run}
        disabled={save.disabled}
        title={save.disabled ? save.reason : undefined}
        className={more.length > 0 ? "rounded-r-none pr-2" : undefined}
      >
        <Save aria-hidden />
        {/* On a phone the toolbar keeps its one row by showing the icon. */}
        <span className="sr-only sm:not-sr-only">Save</span>
      </Button>
      {more.length > 0 ? (
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon"
                aria-label="More ways to save"
                className="w-6 rounded-l-none aria-expanded:bg-muted"
              />
            }
          >
            <ChevronDown className="size-3.5" aria-hidden />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-56">
            {more.map((action) => (
              <ActionItem key={action.id} action={action} />
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  )
}

/**
 * ⋯: the editor's less frequent actions, in groups. Every one of them is in
 * the command menu too; the command menu is an extra way in, not the only one.
 */
export function MoreActionsMenu({
  groups,
}: {
  readonly groups: readonly (readonly StudioAction[])[]
}) {
  const shown = groups.filter((group) => group.length > 0)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            aria-label="More query actions"
            className="aria-expanded:bg-muted"
          />
        }
      >
        <Ellipsis aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        {shown.map((group, index) => (
          <React.Fragment key={group[0]!.id}>
            {index > 0 ? <DropdownMenuSeparator /> : null}
            {group.map((action) => (
              <ActionItem key={action.id} action={action} />
            ))}
          </React.Fragment>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** Shows or hides the Schema | Plan side panel; pressed while it is open. */
export function PanelToggle({
  open,
  onToggle,
}: {
  readonly open: boolean
  readonly onToggle: () => void
}) {
  return (
    <Button
      variant="ghost"
      size="icon"
      aria-pressed={open}
      aria-label={open ? "Hide schema and plan" : "Show schema and plan"}
      title={open ? "Hide schema and plan" : "Show schema and plan"}
      onClick={onToggle}
      className="aria-pressed:bg-muted"
    >
      <PanelRight aria-hidden />
    </Button>
  )
}
