"use client";

import { cn } from "@/lib/utils";

/**
 * A failure message with, when the server gave one, the reference it logged
 * the real error under (SEC-11). The reference is small and selectable so a
 * user can quote it to an administrator; the message itself is the fixed
 * sentence the server chose, never the database's own text.
 */
export function ErrorWithReference({
  message,
  errorId,
  className,
  referenceClassName,
}: {
  message: string;
  errorId?: string | null;
  className?: string;
  referenceClassName?: string;
}) {
  return (
    <span className={cn("block", className)}>
      {message}
      {errorId ? (
        <span className={cn("mt-0.5 block select-text text-[10px] opacity-70", referenceClassName)}>
          Reference: <span className="font-mono">{errorId}</span>
        </span>
      ) : null}
    </span>
  );
}
