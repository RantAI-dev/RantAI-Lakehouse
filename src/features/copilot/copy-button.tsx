"use client";

import * as React from "react";
import { Check, Copy } from "lucide-react";
// `navigator.clipboard` is missing over plain http, so every copy goes
// through the helper that falls back to a textarea (`UX-6`).
import { copyText } from "@/lib/copy-text";
import { notifyFailure } from "@/lib/notify";
import { cn } from "@/lib/utils";

/** Copies `text`; the icon turns into a check for a moment to confirm it. */
export function CopyButton({
  text,
  label = "Copy",
  className,
}: {
  readonly text: string;
  readonly label?: string;
  readonly className?: string;
}) {
  const [copied, setCopied] = React.useState(false);

  React.useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);

  return (
    <button
      type="button"
      onClick={() => {
        // The check mark means "on the clipboard", so it flips only when the
        // helper says the copy worked; a refusal gets the failure toast
        // instead of a success icon (`UX-6` D2).
        void copyText(text).then((copied) => {
          if (copied) {
            setCopied(true);
            return;
          }
          notifyFailure(
            "Could not copy",
            "The browser refused the copy. Select the text and copy it by hand."
          );
        });
      }}
      aria-label={copied ? "Copied" : label}
      title={copied ? "Copied" : label}
      className={cn(
        "inline-flex items-center gap-1 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
        className
      )}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
    </button>
  );
}
