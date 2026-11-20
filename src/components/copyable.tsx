"use client";

import { Copy } from "lucide-react";

import { Button } from "@/components/ui/button";
import { copyText } from "@/lib/copy-text";
// `lib/notify` is this codebase's single door to `sonner` — going straight
// to `toast()` (as the upstream version of this file did) would sidestep
// the shared error-message translation and the `aborted` suppression.
import { notifyFailure, notifySuccess } from "@/lib/notify";
import { cn } from "@/lib/utils";

const TOAST_VALUE_MAX = 48;

interface CopyableProps {
  /** String written to the clipboard. Empty/null skips the copy control. */
  value: string | null | undefined;
  /** What the cell shows. */
  children: React.ReactNode;
  className?: string;
}

function formatCopiedToast(value: string) {
  if (value.length <= TOAST_VALUE_MAX) return `Copied “${value}”`;
  return `Copied “${value.slice(0, TOAST_VALUE_MAX)}…”`;
}

export function Copyable({ value, children, className }: CopyableProps) {
  const copyValue = value?.trim() ? value : null;

  const onCopy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    event.preventDefault();
    if (!copyValue) return;

    // `copyText` answers whether the copy worked; over plain http there is no
    // clipboard API and the textarea fallback may still be refused (`UX-6`),
    // so the success toast is shown only on `true`.
    const copied = await copyText(copyValue);
    if (copied) {
      notifySuccess(formatCopiedToast(copyValue));
    } else {
      notifyFailure(
        "Could not copy",
        "The browser refused the copy. Select the text and copy it by hand."
      );
    }
  };

  return (
    <span
      className={cn(
        "group/copyable inline-flex max-w-full items-center gap-1",
        className
      )}
    >
      <span className="min-w-0 truncate">{children}</span>
      {copyValue ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="Copy"
          className={cn(
            "text-muted-foreground opacity-0 transition-opacity delay-0 duration-150",
            // Delay only on show; leave instantly when the pointer leaves.
            "group-hover/copyable:opacity-100 group-hover/copyable:delay-300",
            "focus-visible:opacity-100 focus-visible:delay-0"
          )}
          onClick={onCopy}
        >
          <Copy />
        </Button>
      ) : null}
    </span>
  );
}
