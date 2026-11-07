import * as React from "react";
import type { Metadata } from "next";
import { getMessages } from "next-intl/server";
import { SkipLink } from "@/components/layout/skip-link";
import { TopBar } from "@/components/layout/top-bar";
import { routing } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";

/**
 * WEB-5 hero explorations (the owner's "3 desain baru"): a shell for the
 * labs at /lab/ and /lab/{n}/, English only, never indexed, in no sitemap and
 * linked from nowhere on the site. Each lab lives in its own folder
 * (`components/lab/{n}/` plus `app/(en)/lab/{n}/`), so removing one is one
 * folder and its route; removing them all is this folder and `app/(en)/lab/`.
 * Only the lab routes import anything here, so the main pages' bundles are
 * untouched.
 */
export function labMetadata(title: string): Metadata {
  return { title: `${title} · Lab · RantAI Lakehouse`, robots: { index: false, follow: false } };
}

export type LabCopy = { cta: CommonCopy["cta"]; contactHref: string };

export async function LabShell({ children }: Readonly<{ children: (copy: LabCopy) => React.ReactNode }>) {
  const locale = routing.defaultLocale;
  const common = (await getMessages({ locale })).common as CommonCopy;
  return (
    <div className="lk-root relative min-h-screen overflow-x-clip bg-[#020514]">
      <SkipLink label={common.skipToContent} />
      <TopBar locale={locale} page="lab" nav={common.nav} cta={common.cta} language={common.language} />
      <main id="main" tabIndex={-1}>
        {children({ cta: common.cta, contactHref: "/contact/" })}
      </main>
    </div>
  );
}
