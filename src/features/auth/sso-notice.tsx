"use client";

import * as React from "react";
import { useSearchParams } from "next/navigation";
import { TriangleAlert } from "lucide-react";

/**
 * The query parameter the API's OIDC callback uses to tell the console
 * that this sign-in succeeded with a caveat (`SEC-39`). It rides on the
 * redirect to the flow cookie's `next` path, so the notice appears on the
 * page the user actually lands on after signing in — the console has no
 * separate "callback result" screen.
 */
export const SSO_NOTICE_PARAM = "ssoNotice";

/**
 * The fixed notice for each code. Only codes listed here are rendered;
 * anything else (an older console, a hand-typed URL) shows nothing. The
 * API sends the code, never the sentence, so the wording lives in one
 * place.
 *
 * `group_overage`: Entra ID replaces the `groups` claim with an overage
 * marker when a user belongs to more groups than fit in the token, so no
 * group-based role could be applied (`docs/OPERATIONS.md`, Microsoft Entra
 * ID → Limits). Following Microsoft Graph to fetch the list is the full
 * fix and is not built.
 */
const NOTICES: Record<string, string> = {
  group_overage:
    "Your account is in more groups than the sign-in token can list, so no group-based role was applied. Ask an administrator to assign your role directly or to limit the groups sent in the token.",
};

/**
 * A one-line notice under the navbar for a sign-in that succeeded with a
 * caveat. Renders nothing otherwise, so it is safe to mount once for the
 * whole authenticated console.
 *
 * Must be rendered under a `<Suspense>` boundary: it reads
 * `useSearchParams()`, which Next.js requires to be wrapped for pages it
 * prerenders (see `AppFrame`).
 */
export function SsoNotice() {
  const params = useSearchParams();
  const code = params.get(SSO_NOTICE_PARAM);
  const text = code ? NOTICES[code] : undefined;
  if (!text) return null;

  return (
    <p
      role="status"
      className="mb-4 flex items-start gap-2 rounded-lg bg-amber-500/10 px-3.5 py-2.5 text-sm text-amber-700 dark:text-amber-400"
    >
      <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
      <span>{text}</span>
    </p>
  );
}
