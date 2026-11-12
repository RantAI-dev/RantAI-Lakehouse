import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import { LINKS } from "./home-content";
import { SITE_NAME } from "./seo";

/**
 * The home page's schema.org data (WEB-5 D4): the product as a
 * `SoftwareApplication`, RantAI as its `Organization`, and the public
 * repository as a `SoftwareSourceCode` whose `targetProduct` is the
 * application. Review fix R3: `codeRepository` is a property of
 * `SoftwareSourceCode`, not of `SoftwareApplication`, where validators flag
 * it.
 *
 * Principle 2: there is no `aggregateRating`, `review` or `offers`, because
 * there are none to state, and none are invented to win a rich result. A
 * test fails if one appears.
 */
export function homeStructuredData({
  locale,
  siteUrl,
  description,
}: {
  locale: Locale;
  siteUrl: string;
  description: string;
}): Record<string, unknown> {
  const url = `${siteUrl}${localizedPath(locale, "/")}`;
  const appId = `${url}#software`;
  const organization = {
    "@type": "Organization",
    "@id": `${LINKS.rantai}/#organization`,
    name: "RantAI",
    url: LINKS.rantai,
  };
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "SoftwareApplication",
        "@id": appId,
        name: SITE_NAME,
        description,
        url,
        inLanguage: locale,
        applicationCategory: "BusinessApplication",
        operatingSystem: "Linux",
        license: "https://www.gnu.org/licenses/agpl-3.0.html",
        isAccessibleForFree: true,
        publisher: { "@id": organization["@id"] },
      },
      {
        "@type": "SoftwareSourceCode",
        name: SITE_NAME,
        codeRepository: LINKS.source,
        license: "https://www.gnu.org/licenses/agpl-3.0.html",
        targetProduct: { "@id": appId },
        publisher: { "@id": organization["@id"] },
      },
      organization,
    ],
  };
}
