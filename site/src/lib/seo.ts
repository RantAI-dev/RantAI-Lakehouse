import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { languageAlternates, localizedPath } from "@/i18n/paths";
import { routing, type Locale } from "@/i18n/routing";

/**
 * Page metadata in one language (WEB-5 D4, shaped like the RantAI site's
 * `lib/seo.ts`): title, description, canonical URL, hreflang alternates and
 * the tags link previews read (Open Graph, X). Paths are relative; the root
 * layout's `metadataBase` (from `NEXT_PUBLIC_SITE_URL`, D2) makes them
 * absolute. The preview image is the `og.png` each language's route
 * handler writes at build time (`lib/og-image.tsx`).
 *
 * Nothing here sets `robots`: the site is meant to be indexed, and no page
 * carries `noindex` (D4).
 */

export const SITE_NAME = "RantAI Lakehouse";

const OG_LOCALE: Record<Locale, string> = { en: "en_US", id: "id_ID" };

export type PageText = { title: string; description: string };

/** The preview card each language's route handler writes (`app/…/og.png`). */
export const OG_IMAGE = { width: 1200, height: 630, type: "image/png" } as const;

export function ogImagePath(locale: Locale): string {
  return `${localizedPath(locale, "/")}og.png`;
}

/** The pure part, so it is unit-tested without a Next build. */
export function buildPageMetadata({
  locale,
  path,
  text,
}: {
  locale: Locale;
  /** Path from the site root, in English: "/" for the home page. */
  path: string;
  text: PageText;
}): Metadata {
  const url = localizedPath(locale, path);
  return {
    // The home page's title already names the product, so the layout's
    // "%s — RantAI Lakehouse" template is not applied to it.
    title: { absolute: text.title },
    description: text.description,
    alternates: { canonical: url, languages: languageAlternates(path) },
    openGraph: {
      type: "website",
      url,
      siteName: SITE_NAME,
      locale: OG_LOCALE[locale],
      alternateLocale: routing.locales.filter((l) => l !== locale).map((l) => OG_LOCALE[l]),
      title: text.title,
      description: text.description,
      images: [{ url: ogImagePath(locale), ...OG_IMAGE, alt: SITE_NAME }],
    },
    twitter: {
      card: "summary_large_image",
      title: text.title,
      description: text.description,
      images: [ogImagePath(locale)],
    },
  };
}

/** Metadata for a page whose title and description are `metadata.<page>` in the messages. */
export async function pageMetadata({
  locale,
  path,
  page = "home",
}: {
  locale: Locale;
  path: string;
  page?: "home";
}): Promise<Metadata> {
  const t = await getTranslations({ locale, namespace: "metadata" });
  return buildPageMetadata({
    locale,
    path,
    text: { title: t(`${page}.title`), description: t(`${page}.description`) },
  });
}
