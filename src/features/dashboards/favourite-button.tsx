"use client";

import { Star } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** The star that marks a dashboard as a favourite (BI-18 part C, part 2). */
export function FavouriteButton({
  on, name, onToggle, className,
}: {
  readonly on: boolean;
  readonly name: string;
  readonly onToggle: () => void;
  readonly className?: string;
}) {
  const label = on ? `Remove ${name} from favourites` : `Add ${name} to favourites`;
  return (
    <Tooltip>
      <TooltipTrigger
        render={<Button variant="ghost" size="icon-sm" aria-label={label} aria-pressed={on} onClick={onToggle} className={className} />}
      >
        <Star className={cn("size-4", on ? "fill-amber-400 text-amber-500" : "text-muted-foreground")} />
      </TooltipTrigger>
      <TooltipContent>{on ? "Favourite" : "Add to favourites"}</TooltipContent>
    </Tooltip>
  );
}
