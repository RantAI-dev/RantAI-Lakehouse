/**
 * RantAI Design System — Font Setup
 *
 * Geist (sans) + Geist Mono (mono) loaded from the files committed beside this
 * module through `next/font/local`, so the console builds with no network
 * (OPS-16, decision D5). Import this in your root layout.tsx and apply the
 * variables to <html>.
 *
 * The files are the release's own webfonts, not a subset: vercel/geist-font
 * ships one variable `woff2` per face (the full character set), so there is no
 * latin-only file to commit and no `subsets` option here. `weight` declares the
 * `wght` axis the variable font carries, which is what the removed Google
 * loader inferred from its request.
 *
 * @example
 * ```tsx
 * // src/app/layout.tsx
 * import { geist, geistMono, fontVariables } from "@/design-system/fonts/fonts";
 *
 * export default function RootLayout({ children }) {
 *   return (
 *     <html className={fontVariables}>
 *       <body>{children}</body>
 *     </html>
 *   );
 * }
 * ```
 */

import localFont from "next/font/local";

export const geist = localFont({
  src: "./geist/Geist-wght.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-sans",
  display: "swap",
  preload: true,
});

export const geistMono = localFont({
  src: "./geist-mono/GeistMono-wght.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-mono",
  display: "swap",
  preload: true,
});

/**
 * Concatenated className string to spread onto <html>.
 * Includes both CSS variable declarations + antialiased.
 */
export const fontVariables = [
  geist.variable,
  geistMono.variable,
  "font-sans",
  "antialiased",
].join(" ");
