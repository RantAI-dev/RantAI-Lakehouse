import type { Metadata, Viewport } from "next";
import { LandingPage } from "@/features/landing/landing-page";
import { displayFont } from "@/features/landing/landing-fonts";

const DESCRIPTION =
  "One lakehouse with dashboards built in. Bring every source in, keep it in open tables, and ask for answers in plain language.";

export const metadata: Metadata = {
  title: "RantAI Lakehouse — your data has a home",
  description: DESCRIPTION,
  openGraph: {
    title: "RantAI Lakehouse — your data has a home",
    description: DESCRIPTION,
    type: "website",
  },
};

/** Browser chrome takes the abyss colour so the page reaches the screen edges. */
export const viewport: Viewport = {
  themeColor: "#020514",
};

/**
 * Public route: `/landing`. Outside the authenticated shell (see `AppFrame`
 * and `PUBLIC_PREFIXES`). The `<noscript>` rule keeps the scroll-reveal
 * sections readable when JavaScript is off.
 */
export default function Page() {
  return (
    <div className={displayFont.variable}>
      <noscript>
        <style>{`[data-reveal]{opacity:1!important;transform:none!important;filter:none!important}`}</style>
      </noscript>
      <LandingPage />
    </div>
  );
}
