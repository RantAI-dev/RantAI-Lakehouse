import type { Metadata } from "next";
import { EmbedView } from "@/features/dashboards/embed-view";

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
  searchParams: Promise<{ chart?: string }>;
}) {
  const { token } = await params;
  const { chart } = await searchParams;
  return <EmbedView token={token} chartId={chart} />;
}
