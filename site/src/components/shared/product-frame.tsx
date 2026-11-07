import type * as React from "react";
import { MascotMark } from "./mascot-svg";

/**
 * A window for a piece of the console, framed in the lake's style (WEB-5
 * D19): the site's glass card and hairline, a slim title strip with the mark
 * and what the window shows, and a lake-blue glow under it like the moon on
 * the water. Not a browser mock-up: no fake address bar and no traffic
 * lights.
 *
 * WEB-5 O22: it frames live HTML (`children`), a recreation of the console
 * that is crisp at any size; the caller labels it as recreated and
 * describes it for assistive tech. The screenshot it used to frame, and the
 * picture branch for it, went with the O22 follow-up once nothing used them.
 */
export function ProductFrame({
  windowTitle,
  className,
  children,
}: Readonly<{
  /** What the window shows, in the title strip ("Home"). */
  windowTitle: string;
  className?: string;
  children: React.ReactNode;
}>) {
  return (
    <div className={className}>
      <div className="relative">
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -inset-x-6 -bottom-10 h-24 rounded-[50%] bg-[radial-gradient(ellipse,rgba(68,166,250,0.28)_0%,transparent_70%)] blur-md"
        />
        <div className="relative overflow-hidden rounded-[22px] bg-[#0B1A4A]/60 shadow-[0_40px_100px_-40px_rgba(2,5,20,0.95)] ring-1 ring-[#E8F4FF]/14 backdrop-blur-md">
          <div className="flex items-center gap-2.5 border-b border-[#E8F4FF]/10 px-4 py-2.5">
            <MascotMark className="h-3.5 w-auto" />
            <span className="text-[12px] leading-4 font-medium text-[#E8F4FF]/75">{windowTitle}</span>
          </div>
          <div className="p-2 sm:p-3">{children}</div>
        </div>
      </div>
    </div>
  );
}
