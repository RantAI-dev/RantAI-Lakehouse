import type { Metadata } from "next";
import { ReportRenderView } from "@/features/dashboards/report-render-view";

export const metadata: Metadata = {
  title: "Report PDF · Rantai Lake",
  robots: { index: false, follow: false },
  referrer: "no-referrer",
};

/**
 * The report's PDF print view (`BI-18` part D, T2): /render/report/<render
 * token>. Opened only by the Gotenberg container, with a token the API minted
 * for the person who asked (2 minutes), exactly as `/render/dashboard/<token>`.
 * AppFrame skips its chrome for `/render`; `proxy.ts` sends its headers for
 * every `/render/*` path.
 */
export default async function RenderReportPage({
  params,
}: {
  params: Promise<{ token: string }>;
}) {
  const { token } = await params;
  return <ReportRenderView token={token} />;
}
