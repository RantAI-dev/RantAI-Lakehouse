import type { Metadata } from "next";
import { RenderView } from "@/features/dashboards/render-view";

export const metadata: Metadata = {
  title: "Chart image · Rantai Lake",
  robots: { index: false, follow: false },
  referrer: "no-referrer",
};

/**
 * The chart image's view (`BI-19` part D, plan D13): /render/chart/<render
 * token>. Opened only by the Gotenberg container, with a token that names one
 * chart (2 minutes). It is under `/render`, so `proxy.ts` applies the same
 * policy as the PDF view (SEC-47: `connect-src 'self'` covers its one call,
 * `POST /api/render/data`; `frame-ancestors 'none'`; `no-store`; `no-referrer`)
 * and nothing is loosened for it.
 */
export default async function RenderChartPage({
  params,
}: {
  params: Promise<{ token: string }>;
}) {
  const { token } = await params;
  return <RenderView token={token} chart />;
}
