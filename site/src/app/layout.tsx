import type { Metadata } from "next";

/**
 * The top layout renders no `<html>` of its own: each language writes its
 * own files with its own `lang` (WEB-5 D3), so the document is rendered one
 * level down by `app/(en)/layout.tsx` and `app/[locale]/layout.tsx` (and by
 * `not-found.tsx`), through `components/layout/site-document.tsx`. This
 * pass-through layout is next-intl's documented shape for a root
 * `not-found` without middleware.
 */
export const metadata: Metadata = {
  title: "RantAI Lakehouse",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return children;
}
