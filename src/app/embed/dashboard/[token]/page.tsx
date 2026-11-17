import type { Metadata } from "next";
import { EmbedView } from "@/features/dashboards/embed-view";
import { parseEmbedLook } from "@/lib/embed-look";

export const metadata: Metadata = {
  title: "Embedded dashboard · Rantai Lake",
  robots: { index: false, follow: false },
};

/**
 * The EMBED route: /embed/dashboard/<token>[?chart=<chartId>]. For an
 * <iframe> on another site. AppFrame skips its chrome; transparent
 * background.
 */
export default async function EmbedDashboardPage({
  params, searchParams,
}: {
  params: Promise<{ token: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { token } = await params;
  const query = await searchParams;
  const chart = Array.isArray(query.chart) ? query.chart[0] : query.chart;
  // BI-26 (decision 12): a public-link embed gets the look and the height messages too.
  return <EmbedView token={token} chartId={chart} look={parseEmbedLook(query)} />;
}
