"use client";

import * as React from "react";
import { Star } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/**
 * The star that marks a dashboard as a favourite (BI-18 part C, part 2).
 *
 * Micro-interaction audit 2026-11-14 (emil, DESIGN.md §3 "no button that sits
 * still when clicked"): the star changes the moment it is pressed, not when
 * the server has answered and the list reloaded. When `onToggle` returns a
 * promise, the star shows the new state until `on` catches up, and goes back
 * if the promise rejects. Turning it on gives the star a small pop.
 */
export function FavouriteButton({
  on, name, onToggle, className,
}: {
  readonly on: boolean;
  readonly name: string;
  readonly onToggle: () => void | Promise<unknown>;
  readonly className?: string;
}) {
  // The state the person asked for while the server has not confirmed it; null when `on` is the truth.
  const [asked, setAsked] = React.useState<boolean | null>(null);
  const shown = asked ?? on;
  // `on` caught up (or moved on its own): the prop is the truth again.
  const [seenOn, setSeenOn] = React.useState(on);
  if (seenOn !== on) {
    setSeenOn(on);
    setAsked(null);
  }
  const toggle = () => {
    const next = !shown;
    setAsked(next);
    const done = onToggle();
    if (done && typeof done.then === "function") {
      done.then(
        () => undefined,
        () => setAsked(null),
      );
    }
  };
  const label = shown ? `Remove ${name} from favourites` : `Add ${name} to favourites`;
  return (
    <Tooltip>
      <TooltipTrigger
        render={<Button variant="ghost" size="icon-sm" aria-label={label} aria-pressed={shown} onClick={toggle} className={className} />}
      >
        <Star
          // A new key when it turns on replays the pop; turning off just fades.
          key={shown ? "on" : "off"}
          className={cn(
            "size-4 transition-[color,fill] duration-200 ease-ui",
            shown ? "animate-pop fill-amber-400 text-amber-500" : "text-muted-foreground",
          )}
        />
      </TooltipTrigger>
      <TooltipContent>{shown ? "Favourite" : "Add to favourites"}</TooltipContent>
    </Tooltip>
  );
}
