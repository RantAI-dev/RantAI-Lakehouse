"use client";

import { usePathname as useNextPathname } from "next/navigation";
import { unprefixedPath } from "./paths";

/**
 * The page's path without its language prefix (`/id/x/` -> `/x/`), for
 * links to the same page in another language (`localizedPath`).
 *
 * Not next-intl's `createNavigation`: its module bundles the code that
 * writes the `NEXT_LOCALE` cookie when a link changes language, and its
 * `Link` points English at `/en/` for a middleware to redirect. The site has
 * neither cookies (WEB-5 D1) nor a middleware (D3), both found in T2.
 */
export function usePathname(): string {
  return unprefixedPath(useNextPathname() ?? "/");
}
