import type { Metadata } from "next";
import { EmbedView } from "@/features/dashboards/embed-view";
import { parseEmbedLook } from "@/lib/embed-look";

export const metadata: Metadata = {
  title: "Embedded dashboard · Rantai Lake",
  robots: { index: false, follow: false },
};

/**
 * SIGNED EMBED: /embed/signed/<jwt>[?chart=<id>][&theme=light|dark|dashboard][&logo=none][&background=transparent]. The
 * look is in the address, not the token (BI-26 D6); a chart token needs no `chart`. The host signs the JWT
 * with the EMBEDDING SECRET; the server verifies it and locks the filters
 * (params). For an <iframe>. AppFrame skips the chrome; transparent
 * background.
 */
export default async function SignedEmbedPage({
  params, searchParams,
}: {
  params: Promise<{ jwt: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { jwt } = await params;
  const query = await searchParams;
  const chart = Array.isArray(query.chart) ? query.chart[0] : query.chart;
  return <EmbedView jwt={jwt} chartId={chart} look={parseEmbedLook(query)} />;
}
