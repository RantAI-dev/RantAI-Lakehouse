import type { Metadata } from "next";
import { EmbedView } from "@/features/dashboards/embed-view";

export const metadata: Metadata = {
  title: "Embedded dashboard · Rantai Lake",
  robots: { index: false, follow: false },
};

/**
 * SIGNED EMBED: /embed/signed/<jwt>[?chart=<id>]. The host signs the JWT
 * with the EMBEDDING SECRET; the server verifies it and locks the filters
 * (params). For an <iframe>. AppFrame skips the chrome; transparent
 * background.
 */
export default async function SignedEmbedPage({
  params, searchParams,
}: {
  params: Promise<{ jwt: string }>;
  searchParams: Promise<{ chart?: string }>;
}) {
  const { jwt } = await params;
  const { chart } = await searchParams;
  return <EmbedView jwt={jwt} chartId={chart} />;
}
