import type { Metadata } from "next";
import { setRequestLocale } from "next-intl/server";
import { routing } from "@/i18n/routing";
import { Lab2Page } from "@/components/lab/2/lab2-page";

/**
 * WEB-5 lab 2, "Typographic reflection": an exploration of the hero for the
 * owner to look at ("buatkan aja di path lain biar aku bisa langsung cek").
 * English only, never indexed, not in the sitemap and not linked from the
 * site. Everything it needs is in `components/lab/2/`, so removing it is
 * that folder and this route.
 */
export const metadata: Metadata = {
  title: "Lab 2 · Typographic reflection",
  robots: { index: false, follow: false },
};

export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return <Lab2Page />;
}
