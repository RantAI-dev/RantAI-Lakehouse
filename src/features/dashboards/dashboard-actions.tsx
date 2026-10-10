"use client";

import * as React from "react";
import {
  Check, Copy, Download, FileDown, Maximize2, Minimize2, MoreHorizontal, Pencil, RefreshCw, Save, Share2, Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuGroupLabel,
  DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { REFRESH_OPTIONS } from "@/lib/dashboard-refresh";

/** The dashboard's ⋯ menu: refresh, view, auto-refresh, and board actions. */
export function DashboardActionsMenu({
  isDefault, loading, fullscreen, autoSec, savedSec, canSaveRefresh, savingRefresh,
  onRefresh, onToggleFullscreen, onAutoSec, onSaveRefresh, onRename, onShare, onExportPdf, onDuplicate, onDelete,
}: {
  readonly isDefault: boolean;
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
  readonly onShare: () => void;
  readonly onExportPdf: () => void;
  readonly onDuplicate: () => void;
  readonly onDelete: () => void;
}) {
  return (
    <div className="relative">
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button variant="outline" size="sm" aria-label="More actions" />}>
          <MoreHorizontal className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuItem onClick={onRefresh} disabled={loading}>
              <RefreshCw className={cn("size-4", loading && "animate-spin")} />
              {loading ? "Refreshing…" : "Refresh now"}
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onToggleFullscreen}>
              {fullscreen ? <Minimize2 className="size-4" /> : <Maximize2 className="size-4" />}
              {fullscreen ? "Exit fullscreen" : "Fullscreen"}
            </DropdownMenuItem>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          {/* Rarely changed, so tucked in here; the active choice is ticked and
              the button carries a dot while an interval is on. */}
          <DropdownMenuGroup>
            <DropdownMenuGroupLabel>Auto-refresh</DropdownMenuGroupLabel>
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
              <DropdownMenuItem onClick={onSaveRefresh} disabled={savingRefresh}>
                <Save className="size-4" /> {savingRefresh ? "Saving…" : "Save as dashboard default"}
              </DropdownMenuItem>
            ) : null}
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            {/* Kept in place on the built-in dashboard rather than removed:
                a name that cannot be changed is worth saying once, whereas a
                menu that quietly grows an entry on every other board leaves
                the reader wondering what else is missing here. The wrapping
                span carries the hover — the disabled item itself has
                `pointer-events-none`. */}
            {isDefault ? (
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
            )}
            {!isDefault ? (
              <DropdownMenuItem onClick={onShare}>
                <Share2 className="size-4" /> Share…
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuItem onClick={onExportPdf}>
              <FileDown className="size-4" /> Export PDF (print)
            </DropdownMenuItem>
            <DropdownMenuItem render={<a href="/api/dashboard/export" download />}>
              <Download className="size-4" /> Export YAML
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onDuplicate}>
              <Copy className="size-4" /> Duplicate
            </DropdownMenuItem>
          </DropdownMenuGroup>
          {!isDefault ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem variant="destructive" onClick={onDelete}>
                  <Trash2 className="size-4" /> Delete dashboard
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      {autoSec !== "0" ? (
        <span className="pointer-events-none absolute -right-0.5 -top-0.5 size-2 rounded-full bg-primary ring-2 ring-background" aria-hidden />
      ) : null}
    </div>
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
