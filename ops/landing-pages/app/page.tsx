import { LandingPage } from "@/features/landing/landing-page";

/**
 * The public landing page, served at `/` by GitHub Pages
 * (`https://lakehouse.rantai.dev`).
 *
 * `LandingPage` is the same component `src/app/landing/page.tsx` renders in
 * the console's dev server, so the two pages cannot drift: this wrapper only
 * supplies the root route and the `<noscript>` reveal fallback, exactly as
 * that page does.
 */
export default function Page() {
  return (
    <>
      <noscript>
        <style>{`[data-reveal]{opacity:1!important;transform:none!important;filter:none!important}`}</style>
      </noscript>
      <LandingPage />
    </>
  );
}
