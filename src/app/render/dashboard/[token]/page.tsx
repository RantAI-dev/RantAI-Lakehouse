import type { Metadata } from "next";
import { RenderView } from "@/features/dashboards/render-view";

export const metadata: Metadata = {
  title: "Dashboard PDF · Rantai Lake",
  robots: { index: false, follow: false },
  referrer: "no-referrer",
};

/**
 * The PDF print view (`BI-1`): /render/dashboard/<render token>. Opened only by
 * the Gotenberg container, with a token the API minted for the person who asked
 * (2 minutes). AppFrame skips its chrome for `/render`; `proxy.ts` sends
 * `frame-ancestors 'none'`, `no-store` and `no-referrer` for this path.
 */
export default async function RenderDashboardPage({
  params,
}: {
  params: Promise<{ token: string }>;
}) {
  const { token } = await params;
  return <RenderView token={token} />;
}
