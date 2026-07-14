"use client";

import { ThemeProvider as NextThemesProvider } from "next-themes";

/**
 * Thin wrapper around `next-themes` ThemeProvider.
 *
 * Lives on the client side so the root layout (a server component) can opt into
 * dark/light/system mode without becoming a client component itself.
 * Props are forwarded as-is to `NextThemesProvider`.
 */
export function ThemeProvider({
  children,
  ...props
}: React.ComponentProps<typeof NextThemesProvider>) {
  return <NextThemesProvider {...props}>{children}</NextThemesProvider>;
}
