/**
 * schema.org `FAQPage` data for the contact page's questions (WEB-5 D15, the
 * RantAI site's `lib/faq-json-ld.ts`). It carries the same questions and
 * answers the page shows, so it claims nothing the page does not: each
 * answer is a claim traced in `content/claims.ts`.
 */
export function faqJsonLd(items: readonly { q: string; a: string }[]): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: items.map((item) => ({
      "@type": "Question",
      name: item.q,
      acceptedAnswer: { "@type": "Answer", text: item.a },
    })),
  };
}
