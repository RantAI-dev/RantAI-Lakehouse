"use client";

import * as React from "react";
import {
  Check, Clock, Copy, Download, FileDown, History, LayoutTemplate, Mail, Printer, Maximize2, Minimize2, Palette, Pencil, Plus, RefreshCw, Save, Settings, Timer, Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuGroupLabel,
  DropdownMenuItem, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubContent, DropdownMenuSubTrigger, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { REFRESH_OPTIONS } from "@/lib/dashboard-refresh";
import { PDF_NOT_SET_UP } from "./chart-download";

/**
 * The dashboard's ⋯ menu (`BI-61` T2, spec T2): one menu in three groups,
 * View, Get a copy and Manage, with the seven auto-refresh choices in one
 * submenu and the two exports in another, so it lists at most 14 entries at
 * its top level where it used to list 22 flat ones. Share is not here: it is
 * the page's primary button (`ShareButton`). Every other entry the old menu
 * had is still here, under the same rules.
 */
export function DashboardActionsMenu({
  isDefault, canWrite, onHistory, onTheme, onFreshness, loading, fullscreen, autoSec, savedSec, canSaveRefresh, savingRefresh,
  onRefresh, onToggleFullscreen, onAutoSec, onSaveRefresh, onRename, onExportPdf, onDuplicate, onDelete,
  onDownloadPdf, pdfSupported = true, canDownload = true, onSubscriptions, onExportFile, onSaveTemplate, canManage = canWrite, dataFrom,
}: {
  readonly isDefault: boolean;
  /**
   * The caller holds `dashboard:write`. Without it Rename, Duplicate and Delete
   * are left out, not shown to fail on click (BI-18 part C review fix
   * (SHOULD-FIX) R18).
   */
  readonly canWrite: boolean;
  /**
   * The caller manages the dashboard (`BI-19` part B2): only then is Delete
   * offered. Absent: as `canWrite`, the answer before levels existed.
   */
  readonly canManage?: boolean;
  /** Opens the version history; the item is left out when absent. */
  readonly onHistory?: () => void;
  /** Opens "Theme and font…" (`BI-17`); the item is left out when absent. */
  readonly onTheme?: () => void;
  /** Opens "Freshness…" (`BI-4` part B): how long this dashboard's results are remembered; left out when absent. */
  readonly onFreshness?: () => void;
  readonly loading: boolean;
  readonly fullscreen: boolean;
  /** The interval in force, in seconds ("0" = manual). */
  readonly autoSec: string;
  /** The board's saved default, in seconds. */
  readonly savedSec: string;
  /** An editor whose choice differs from the saved default. */
  readonly canSaveRefresh: boolean;
  readonly savingRefresh: boolean;
  readonly onRefresh: () => void;
  readonly onToggleFullscreen: () => void;
  readonly onAutoSec: (value: string) => void;
  readonly onSaveRefresh: () => void;
  readonly onRename: () => void;
  /** The browser's print dialog ("Print…"); always offered. */
  readonly onExportPdf: () => void;
  /** Opens "Download PDF…" (`BI-1`); the item is left out when absent. */
  readonly onDownloadPdf?: () => void;
  /** False when the deployment has no PDF renderer: the item stays, says why, and cannot be chosen. */
  readonly pdfSupported?: boolean;
  /** False when every role of the viewer is switched off from downloading: the download items are hidden. */
  readonly canDownload?: boolean;
  /** Opens "Subscriptions…" (`RPT-1`); left out when absent or when downloads are switched off for the viewer. */
  readonly onSubscriptions?: () => void;
  /** Downloads the dashboard as a file to import elsewhere (`BI-19` part D); left out when absent. */
  readonly onExportFile?: () => void;
  /** Opens "Save as template" (`BI-18` part D, T6); left out when absent, without edit, when downloads are off, and on the built-in dashboard. */
  readonly onSaveTemplate?: () => void;
  readonly onDuplicate: () => void;
  readonly onDelete: () => void;
  /** "12:47", the time the data on screen is from, shown beside Refresh now. */
  readonly dataFrom?: string | null;
}) {
  const autoLabel = REFRESH_OPTIONS.find((o) => String(o.seconds) === autoSec)?.label ?? "Manual";
  const exportFile = canDownload && !isDefault && onExportFile;
  return (
    <div className="relative">
      <DropdownMenu>
        {/* BI-61 review round 1, SHOULD-FIX 10 (owner): the menu is the dashboard's settings, a gear
            between Edit and Share; 32 px like the console's other page headers (SHOULD-FIX 9). */}
        <Tooltip>
          <DropdownMenuTrigger
            render={
              <TooltipTrigger
                render={<Button variant="ghost" size="icon" aria-label="Dashboard settings" className="text-muted-foreground hover:text-foreground aria-expanded:text-foreground" />}
              />
            }
          >
            <Settings className="size-4" aria-hidden />
          </DropdownMenuTrigger>
          <TooltipContent>Dashboard settings</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="end" className="w-60">
          <DropdownMenuGroup>
            <DropdownMenuGroupLabel>View</DropdownMenuGroupLabel>
            <DropdownMenuItem onClick={onRefresh} disabled={loading}>
              <RefreshCw className={cn("size-4", loading && "animate-spin")} />
              {loading ? "Refreshing…" : "Refresh now"}
              {!loading && dataFrom ? <span className="ml-auto text-xs text-muted-foreground tabular-nums">{dataFrom}</span> : null}
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onToggleFullscreen}>
              {fullscreen ? <Minimize2 className="size-4" /> : <Maximize2 className="size-4" />}
              {fullscreen ? "Exit fullscreen" : "Fullscreen"}
            </DropdownMenuItem>
            {/* The seven intervals in one submenu; the trigger says the one in force. */}
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                {/* BI-61 review round 1, SHOULD-FIX 6: the label grows, so the value sits right beside
                    the chevron (the base trigger's own `ml-auto` chevron would otherwise split the space). */}
                <Timer /> <span className="flex-1">Auto-refresh</span>
                <span data-slot="menu-value" className="text-xs text-muted-foreground">{autoLabel === "Manual" ? "Off" : autoLabel.replace("Every ", "")}</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="w-52">
                {REFRESH_OPTIONS.map((opt) => {
                  const value = String(opt.seconds);
                  return (
                    // Stays open so the "Save as dashboard default" row that a
                    // new choice brings up is seen (product owner QA, BI-18·B).
                    <DropdownMenuItem key={value} closeOnClick={false} onClick={() => onAutoSec(value)}>
                      <Check className={cn("size-4", autoSec === value ? "opacity-100" : "opacity-0")} aria-hidden />
                      {opt.label}
                      {savedSec === value && savedSec !== "0" ? <span className="ml-auto text-[11px] text-muted-foreground">default</span> : null}
                    </DropdownMenuItem>
                  );
                })}
                {canSaveRefresh ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem onClick={onSaveRefresh} disabled={savingRefresh}>
                      <Save className="size-4" /> {savingRefresh ? "Saving…" : "Save as dashboard default"}
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            {!isDefault && onFreshness ? (
              <DropdownMenuItem onClick={onFreshness}>
                <Clock className="size-4" /> Freshness…
              </DropdownMenuItem>
            ) : null}
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuGroupLabel>Get a copy</DropdownMenuGroupLabel>
            {canDownload && onDownloadPdf ? (
              pdfSupported ? (
                <DropdownMenuItem onClick={onDownloadPdf}>
                  <FileDown className="size-4" /> Download PDF…
                </DropdownMenuItem>
              ) : (
                // Kept and disabled, with the reason, like Rename on the built-in
                // dashboard: a missing item would leave the reader wondering why.
                <Tooltip>
                  <TooltipTrigger render={<span className="block" />}>
                    <DropdownMenuItem disabled>
                      <FileDown className="size-4" /> Download PDF…
                    </DropdownMenuItem>
                  </TooltipTrigger>
                  <TooltipContent side="left" className="max-w-56">
                    {PDF_NOT_SET_UP}. Print… still works.
                  </TooltipContent>
                </Tooltip>
              )
            ) : null}
            <DropdownMenuItem onClick={onExportPdf}>
              <Printer className="size-4" /> Print…
            </DropdownMenuItem>
            {canDownload && onSubscriptions ? (
              <DropdownMenuItem onClick={onSubscriptions}>
                <Mail className="size-4" /> Subscriptions…
              </DropdownMenuItem>
            ) : null}
            {/* BI-19 Q-D1 (owner decision 2026-10-27): both exports follow the per-role download switch. */}
            {canDownload ? (
              <DropdownMenuSub>
                <DropdownMenuSubTrigger><Download /> Export</DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-48">
                  {exportFile ? (
                    <DropdownMenuItem onClick={onExportFile}>Dashboard file</DropdownMenuItem>
                  ) : null}
                  <DropdownMenuItem render={<a href="/api/dashboard/export" download />}>YAML</DropdownMenuItem>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            ) : null}
          </DropdownMenuGroup>
          <ManageGroup
            isDefault={isDefault} canWrite={canWrite} canManage={canManage} canDownload={canDownload}
            onRename={onRename} onDuplicate={onDuplicate} onTheme={onTheme} onHistory={onHistory}
            onSaveTemplate={onSaveTemplate} onDelete={onDelete}
          />
        </DropdownMenuContent>
      </DropdownMenu>
      {autoSec !== "0" ? (
        <span className="pointer-events-none absolute right-1 top-1 size-2 rounded-full bg-primary ring-2 ring-background" aria-hidden />
      ) : null}
    </div>
  );
}

/** The Manage group; drawn only when it holds something (a reader without edit may have nothing to manage). */
function ManageGroup({
  isDefault, canWrite, canManage, canDownload, onRename, onDuplicate, onTheme, onHistory, onSaveTemplate, onDelete,
}: {
  readonly isDefault: boolean;
  readonly canWrite: boolean;
  readonly canManage: boolean;
  readonly canDownload: boolean;
  readonly onRename: () => void;
  readonly onDuplicate: () => void;
  readonly onTheme?: () => void;
  readonly onHistory?: () => void;
  readonly onSaveTemplate?: () => void;
  readonly onDelete: () => void;
}) {
  const theme = !isDefault && onTheme;
  const history = !isDefault && onHistory;
  // BI-18 D review round 1, BLOCKER 1: a template holds what the dashboard file holds, so it follows the download switch too.
  const template = canWrite && canDownload && !isDefault && onSaveTemplate;
  const remove = !isDefault && canManage;
  if (!canWrite && !theme && !history && !template && !remove) return null;
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuGroupLabel>Manage</DropdownMenuGroupLabel>
        {canWrite ? (
          isDefault ? (
            // Kept in place on the built-in dashboard rather than removed: a
            // name that cannot be changed is worth saying once. The wrapping
            // span carries the hover; the disabled item has `pointer-events-none`.
            <Tooltip>
              <TooltipTrigger render={<span className="block" />}>
                <DropdownMenuItem disabled>
                  <Pencil className="size-4" /> Rename
                </DropdownMenuItem>
              </TooltipTrigger>
              <TooltipContent side="left" className="max-w-56">
                The built-in dashboard keeps its name. Duplicate it for a copy you can rename.
              </TooltipContent>
            </Tooltip>
          ) : (
            <DropdownMenuItem onClick={onRename}>
              <Pencil className="size-4" /> Rename
            </DropdownMenuItem>
          )
        ) : null}
        {canWrite ? (
          <DropdownMenuItem onClick={onDuplicate}>
            <Copy className="size-4" /> Duplicate
          </DropdownMenuItem>
        ) : null}
        {theme ? (
          <DropdownMenuItem onClick={onTheme}>
            <Palette className="size-4" /> Theme and font…
          </DropdownMenuItem>
        ) : null}
        {history ? (
          <DropdownMenuItem onClick={onHistory}>
            <History className="size-4" /> Version history…
          </DropdownMenuItem>
        ) : null}
        {template ? (
          <DropdownMenuItem onClick={onSaveTemplate}>
            <LayoutTemplate className="size-4" /> Save as template…
          </DropdownMenuItem>
        ) : null}
        {remove ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onClick={onDelete}>
              <Trash2 className="size-4" /> Delete dashboard
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuGroup>
    </>
  );
}

export function RenameDashboardDialog({
  open, onOpenChange, currentName, onSave,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly currentName: string;
  readonly onSave: (name: string) => void;
}) {
  const [name, setName] = React.useState(currentName);
  const clean = name.trim();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>Rename dashboard</DialogTitle></DialogHeader>
        <Input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && clean) onSave(clean); }}
          placeholder="Dashboard name"
          aria-label="Dashboard name"
        />
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" disabled={!clean} onClick={() => onSave(clean)}>Save</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * "Add chart", the primary button of edit mode and of the empty dashboard
 * (`BI-61` T5); left out without `dashboard:write` (R18).
 */
export function NewChartButton({ canWrite, onClick, size = "default" }: { readonly canWrite: boolean; readonly onClick: () => void; readonly size?: "sm" | "default" }) {
  if (!canWrite) return null;
  return (
    <Button size={size} onClick={onClick}>
      <Plus data-icon="inline-start" aria-hidden /> Add chart
    </Button>
  );
}
