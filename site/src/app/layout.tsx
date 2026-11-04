import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "RantAI Lakehouse",
};

/** Placeholder root layout (WEB-5 T1); T2 replaces it with the localized one. */
export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
