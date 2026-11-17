"use client";

import * as React from "react";
import { SITE_NO_LONGER_ALLOWED, siteAllowed, type TypeDefFields } from "@/lib/type-b";
import { useReporting } from "./reporting-context";

/**
 * The image and web-page cards (`BI-16` part B, decisions 4 to 7). Both load a
 * page or a picture from an address an editor typed, so both check it twice:
 * the server at save, and here at render against the administrator's list as
 * this read gave it (`reporting.contentSites`, the same in the console, a public
 * link and a signed embed). A site removed from the list since the card was
 * saved renders the message and mounts no `img` and no `iframe`, so nothing is
 * requested. Since `SEC-47` the browser backs this up as well: the page's
 * `Content-Security-Policy` limits `img-src` and `frame-src` to those same
 * sites, so a redirect from an allowed site is refused too.
 */

function Blocked({ message }: { readonly message: string }) {
  return <p role="status" className="grid h-full place-items-center px-3 text-center text-xs text-muted-foreground">{message}</p>;
}

export function ImageCard({ def, title }: { readonly def: TypeDefFields | undefined; readonly title: string }) {
  const { contentSites } = useReporting();
  const url = def?.url;
  const [failed, setFailed] = React.useState(false);
  // A new address is a new try.
  React.useEffect(() => setFailed(false), [url]);
  if (!url || !siteAllowed(url, contentSites)) return <Blocked message={SITE_NO_LONGER_ALLOWED} />;
  // The message never repeats the address.
  if (failed) return <Blocked message="The image could not be loaded" />;
  return (
    // eslint-disable-next-line @next/next/no-img-element -- an address on an allowed site; `next/image` would proxy it through the console
    <img
      src={url} alt={def?.alt ?? title} referrerPolicy="no-referrer" loading="lazy"
      className="h-full w-full object-contain" onError={() => setFailed(true)}
    />
  );
}

export function WebCard({ def, title }: { readonly def: TypeDefFields | undefined; readonly title: string }) {
  const { contentSites } = useReporting();
  const url = def?.url;
  if (!url || !siteAllowed(url, contentSites)) return <Blocked message={SITE_NO_LONGER_ALLOWED} />;
  return (
    <div className="flex h-full min-h-0 flex-col gap-1">
      {/*
        Scripts and pop-ups are allowed so a status page works; `allow-same-origin`
        and `allow-top-navigation` are never added, so the page runs as a stranger:
        it cannot read the console's cookies or storage and cannot send the
        window elsewhere. No referrer is sent.
      */}
      <iframe
        src={url} title={title} sandbox="allow-scripts allow-popups" referrerPolicy="no-referrer" loading="lazy"
        className="min-h-0 w-full flex-1 rounded border-0 bg-background"
      />
      {/* Many sites refuse to be framed; the card then stays blank, so the link is always offered. */}
      <a href={url} target="_blank" rel="noopener noreferrer" className="shrink-0 self-end text-xs text-primary underline-offset-2 hover:underline">
        Open in a new tab
      </a>
    </div>
  );
}
