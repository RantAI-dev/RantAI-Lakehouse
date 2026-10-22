"use client";

import { Lock } from "lucide-react";

import { Pill } from "@/components/patterns/status-badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/** What the Private badge says on hover (BI-18 part C review fix (SHOULD-FIX) R11). */
export const PRIVATE_TIP = "Only you and administrators can see it. Move it to a folder to share it.";

/** What the New dashboard dialog says under a Folder that is the personal space. */
export const PRIVATE_HINT = "Private: only you and administrators can see it. It cannot be shared until you move it to a folder.";

/** The lock and "Private" badge of a dashboard in a personal space, as on the list card and beside the title of the dashboard. */
export function PrivateBadge() {
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="inline-flex" />}>
        <Pill tone="neutral">
          <Lock className="size-3" aria-hidden /> Private
        </Pill>
      </TooltipTrigger>
      <TooltipContent>{PRIVATE_TIP}</TooltipContent>
    </Tooltip>
  );
}
