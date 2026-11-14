"use client";

/**
 * The header of the page that shows one dashboard (`BI-61`, layout A of the
 * 2026-11-14 mockups, "Focused reader"): a breadcrumb, the title with its
 * star, one quiet line of facts, and on the right the few controls a reader
 * needs. Reading mode shows at most three (spec T1), left to right: Edit (a
 * pencil, for writers), the dashboard's settings (a gear, the grouped menu)
 * and Share, the one primary button, last (BI-61 review round 1, SHOULD-FIX
 * 10). Everything else the old
 * header carried moved: Comments to the facts line, Parameters and New chart
 * to edit mode, and Stop to the facts line while the tiles load.
 */
import * as React from "react";
import Link from "next/link";
import { ChevronRight, Info, MessageSquare, Pencil, Share2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** One easing and duration for every hover, press and change here (owner taste: 150–250 ms, eased, never linear). */
export const EASE = "transition-[background-color,border-color,color,box-shadow,opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)]";
/** The keyboard focus ring the custom controls of the page use. */
export const FOCUS = "outline-none focus-visible:ring-3 focus-visible:ring-ring/50";

/**
 * An icon button with its name as the tooltip, for the header (the console's
 * `Button`, ghost, so it reads as quiet beside Share), at the console's
 * default 32 px like every other page (BI-61 review round 1, SHOULD-FIX 9).
 */
export function HeaderIconButton({
  label, onClick, children, className, disabled,
}: {
  readonly label: string;
  readonly onClick?: () => void;
  readonly children: React.ReactNode;
  readonly className?: string;
  readonly disabled?: boolean;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button variant="ghost" size="icon" aria-label={label} onClick={onClick} disabled={disabled}
            className={cn("text-muted-foreground hover:text-foreground active:scale-95", EASE, className)} />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/** The pencil that turns the page into edit mode; left out for a caller who cannot edit (R18). */
export function EditPencil({ canWrite, onEdit }: { readonly canWrite: boolean; readonly onEdit: () => void }) {
  if (!canWrite) return null;
  return (
    <HeaderIconButton label="Edit layout" onClick={onEdit}>
      <Pencil className="size-4" aria-hidden />
    </HeaderIconButton>
  );
}

/** What the disabled Share says on a dashboard in a personal space. */
export const SHARE_PRIVATE_TIP = "A dashboard in your personal space cannot be shared. Move it to a folder first.";

/**
 * Share, the page's one primary button. The rules are the ones the ⋯ menu's
 * "Share…" had (`BI-19` B2, BI-18 part C R11): the built-in dashboard has
 * none, a dashboard in a personal space shows it disabled with the reason,
 * and only someone who manages the dashboard is offered it.
 */
export function ShareButton({
  isDefault, isPrivate, canManage, onShare,
}: {
  readonly isDefault: boolean;
  readonly isPrivate: boolean;
  readonly canManage: boolean;
  readonly onShare: () => void;
}) {
  if (isDefault) return null;
  if (isPrivate) {
    return (
      <Tooltip>
        {/* The wrapper carries the hover: a disabled button takes no pointer events. */}
        <TooltipTrigger render={<span className="inline-flex" tabIndex={0} />}>
          <Button disabled aria-label="Share">
            <Share2 data-icon="inline-start" aria-hidden /> Share
          </Button>
        </TooltipTrigger>
        <TooltipContent className="max-w-56">{SHARE_PRIVATE_TIP}</TooltipContent>
      </Tooltip>
    );
  }
  if (!canManage) return null;
  return (
    <Button onClick={onShare} className={cn("active:scale-[0.98]", EASE)}>
      <Share2 data-icon="inline-start" aria-hidden /> Share
    </Button>
  );
}

export type HeaderFacts = {
  /** Who made the dashboard (`Board.createdBy`); absent for an older board. */
  readonly owner?: string;
  /** "Data from 12:47"; absent while nothing has a time. */
  readonly dataFrom?: string | null;
  /** The number of comments; null while not known, undefined on the built-in dashboard. */
  readonly comments?: number | null;
};

/**
 * The breadcrumb, the title row and the facts line. `actions` sit on the
 * right of the title on a laptop and under the facts on a phone.
 */
export function DashboardHeader({
  folderPath, title, titleAccessory, facts, loading, onStop, onComments, onDetails, actions, notice,
}: {
  /** Opens the Details tab of the side panel (sharing, freshness, subscriptions, versions); absent on the built-in dashboard. */
  readonly onDetails?: () => void;
  /** The folder names from the root, empty when the dashboard is in no folder. */
  readonly folderPath: readonly string[];
  /**
   * The dashboard's name; null while it is not known yet (the session check, the first read):
   * the title's line then holds a grey bar of its size, never a stand-in name that changes.
   */
  readonly title: string | null;
  /** The star and the Private badge, beside the title. */
  readonly titleAccessory?: React.ReactNode;
  readonly facts: HeaderFacts;
  /** While the tiles load, the facts line says so and offers Stop (BI-5 decision 4) instead of the data time. */
  readonly loading: boolean;
  readonly onStop: () => void;
  /** Opens the Comments tab of the side panel; absent on the built-in dashboard. */
  readonly onComments?: () => void;
  readonly actions: React.ReactNode;
  /** A line under the title row, e.g. the edit-mode hint. */
  readonly notice?: React.ReactNode;
}) {
  const parts: React.ReactNode[] = [];
  if (facts.owner) parts.push(<span key="owner">{facts.owner}</span>);
  if (loading) {
    parts.push(
      <span key="loading" className="inline-flex items-center gap-1">
        Loading…
        <button type="button" onClick={onStop}
          className={cn("rounded-sm font-medium text-foreground underline-offset-4 hover:underline", EASE, FOCUS)}>
          Stop
        </button>
      </span>,
    );
  } else if (facts.dataFrom) {
    parts.push(<span key="data" className="tabular-nums">{facts.dataFrom}</span>);
  }
  if (onComments && facts.comments !== undefined) {
    const n = facts.comments;
    parts.push(
      <button key="comments" type="button" onClick={onComments}
        className={cn("inline-flex items-center gap-1 rounded-sm underline-offset-4 hover:text-foreground hover:underline", EASE, FOCUS)}>
        <MessageSquare className="size-3.5" aria-hidden />
        {n === null ? "Comments" : `${n.toLocaleString()} comment${n === 1 ? "" : "s"}`}
      </button>,
    );
  }
  if (onDetails) {
    parts.push(
      <button key="details" type="button" onClick={onDetails}
        className={cn("inline-flex items-center gap-1 rounded-sm underline-offset-4 hover:text-foreground hover:underline", EASE, FOCUS)}>
        <Info className="size-3.5" aria-hidden /> Details
      </button>,
    );
  }
  return (
    <header className="grid shrink-0 grid-cols-1 gap-3">
      <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground" data-print-hide>
        <Link href="/dashboards/browse" className={cn("shrink-0 rounded-sm hover:text-foreground", EASE, FOCUS)}>Dashboards</Link>
        {folderPath.map((name, i) => (
          <React.Fragment key={`${i}:${name}`}>
            <ChevronRight className="size-3 shrink-0" aria-hidden />
            <span className="truncate">{name}</span>
          </React.Fragment>
        ))}
      </nav>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
        <div className="grid w-full min-w-0 gap-1 sm:w-auto sm:flex-1">
          <div className="flex min-w-0 items-start gap-1.5 sm:items-center">
            <h1 className="line-clamp-2 min-w-0 text-xl font-semibold tracking-tight text-foreground sm:truncate sm:text-2xl">
              {title ?? (
                // Owner QA 2026-11-14: the name is not known yet, so a bar where it will be, at the
                // title's own line height, rather than "Dashboards" switching to the real name.
                <>
                  <span className="sr-only">Loading dashboard</span>
                  <span aria-hidden data-slot="title-placeholder" className="inline-block h-6 w-56 max-w-full animate-pulse rounded-md bg-muted align-middle" />
                </>
              )}
            </h1>
            {titleAccessory ? <span className="flex shrink-0 items-center gap-1.5" data-print-hide>{titleAccessory}</span> : null}
          </div>
          {/* The facts line keeps its line while it has nothing to say yet (owner QA 2026-11-14:
              the meta row's height stays the same from the first loading frame to the loaded page). */}
          <p className="flex min-h-5 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-sm text-muted-foreground">
            {parts.length === 0 && title === null ? (
              <span aria-hidden data-slot="facts-placeholder" className="inline-block h-3.5 w-64 max-w-full animate-pulse rounded-md bg-muted" />
            ) : null}
            {parts.map((p, i) => (
              <React.Fragment key={i}>
                {i > 0 ? <span aria-hidden>·</span> : null}
                {p}
              </React.Fragment>
            ))}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5" data-slot="dashboard-header-actions" data-print-hide>{actions}</div>
      </div>
      {notice}
    </header>
  );
}
