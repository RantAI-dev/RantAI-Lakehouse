"use client";

import * as React from "react";
import { Menu, X } from "lucide-react";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { NAV_ANCHORS } from "@/lib/home-content";
import { LanguageSwitcher } from "./language-switcher";
import { CtaLink } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";

/**
 * S0 — the fixed top bar, in the RantAI family's layout (WEB-5 owner QA
 * O6: rantai-page `components/layout/navbar.tsx`, in the lake's look), so
 * every product site has the same bar:
 * - an h-16 bar in a 1440 px container: the mark and wordmark, then the
 *   links as pills (a muted hover, and a lit pill for the current item);
 * - at the right, the EN · ID switch and one primary button, "Install it",
 *   which goes to the page's own "Get started" (D20);
 * - below `lg`, a menu button opens a sheet with the links, Contact and the
 *   language switch. Phones used to lose the section links entirely.
 *
 * Transparent over the hero; glass and a foam hairline once the page has
 * scrolled 40 px. On the home page the links are in-page anchors and the
 * current section lights its pill as you scroll (an IntersectionObserver
 * writing `aria-current`, so scrolling never re-renders the bar, D5). On
 * the contact page they lead back to the home page's sections in the
 * visitor's language, and Contact is the current page (D15).
 *
 * The sheet is the native modal `<dialog>`: the browser makes the page
 * under it inert and closes it on Esc, and Tab wraps inside it; the page's
 * scroll is locked while it is open, and it slides in and out with CSS
 * (`.lk-sheet`, app/globals.css), still under reduced motion.
 */
/** `lab`: the WEB-5 hero explorations (`app/(en)/lab/`); links lead to the home page, nothing is current. */
export type TopBarPage = "home" | "contact" | "lab";

const pill =
  "inline-block rounded-full px-3.5 py-2 text-[14px] text-[#E8F4FF]/70 transition-[background-color,color,transform] duration-200 ease-out hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] active:scale-[0.97] active:bg-[#E8F4FF]/[0.14] aria-[current]:bg-[#E8F4FF]/[0.12] aria-[current]:text-[#E8F4FF] motion-reduce:transition-none";
const sheetLink =
  "flex min-h-14 items-center justify-between border-b border-[#E8F4FF]/10 text-lg font-medium text-[#E8F4FF]/60 transition-colors duration-200 ease-out hover:text-[#E8F4FF] active:text-[#8fcbff] aria-[current]:text-[#E8F4FF]";

export function TopBar({
  locale,
  page = "home",
  nav,
  cta,
  language,
}: {
  locale: Locale;
  page?: TopBarPage;
  nav: CommonCopy["nav"];
  cta: CommonCopy["cta"];
  language: CommonCopy["language"];
}) {
  const home = localizedPath(locale, "/");
  const contact = localizedPath(locale, "/contact");
  const anchor = (hash: string) => (page === "home" ? hash : `${home}${hash}`);
  const sheetId = React.useId();

  // WEB-5 D5: no React state per scroll event. The scrolled look is a
  // `data-scrolled` attribute written straight to the header, and only when
  // it flips, so scrolling never re-renders the bar. A plain passive
  // listener rather than motion's `useScroll`, whose scroll-timeline code
  // was about 4 KB of the first JavaScript for one comparison (measured).
  const headerRef = React.useRef<HTMLElement>(null);
  const dialogRef = React.useRef<HTMLDialogElement>(null);
  const buttonRef = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => {
    const apply = () => {
      const el = headerRef.current;
      if (!el) return;
      const next = window.scrollY > 40;
      if (el.hasAttribute("data-scrolled") !== next) el.toggleAttribute("data-scrolled", next);
    };
    apply();
    window.addEventListener("scroll", apply, { passive: true });
    return () => window.removeEventListener("scroll", apply);
  }, []);

  // O6, the scroll spy (home only): the section crossing the middle band of
  // the screen lights its item, in the bar and in the sheet.
  React.useEffect(() => {
    if (page !== "home") return;
    const header = headerRef.current;
    const sheet = dialogRef.current;
    if (!header) return;
    const owner = new Map<string, string>();
    for (const a of NAV_ANCHORS) for (const id of a.spy) owner.set(id, a.href);
    const order = NAV_ANCHORS.flatMap((a) => a.spy);
    const visible = new Set<string>();
    let current: string | null = null;
    const paint = () => {
      const id = order.filter((s) => visible.has(s)).at(-1) ?? null;
      const href = id ? (owner.get(id) ?? null) : null;
      if (href === current) return;
      current = href;
      for (const root of [header, sheet]) {
        root?.querySelectorAll<HTMLAnchorElement>("a[data-spy]").forEach((link) => {
          if (link.dataset.spy === href) link.setAttribute("aria-current", "location");
          else link.removeAttribute("aria-current");
        });
      }
    };
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) visible.add(e.target.id);
          else visible.delete(e.target.id);
        }
        paint();
      },
      { rootMargin: "-45% 0px -50% 0px" },
    );
    for (const id of order) {
      const el = document.getElementById(id);
      if (el) io.observe(el);
    }
    return () => io.disconnect();
  }, [page]);

  // The sheet.
  const openSheet = () => {
    const d = dialogRef.current;
    if (!d || d.open) return;
    d.showModal();
    document.documentElement.style.overflow = "hidden";
    buttonRef.current?.setAttribute("aria-expanded", "true");
  };
  const closeSheet = () => dialogRef.current?.close();
  React.useEffect(() => {
    const d = dialogRef.current;
    if (!d) return;
    // Esc, a link, the close button and a click on the backdrop all end here.
    const onClose = () => {
      document.documentElement.style.overflow = "";
      buttonRef.current?.setAttribute("aria-expanded", "false");
      buttonRef.current?.focus({ preventScroll: true });
    };
    const onBackdrop = (e: MouseEvent) => {
      if (e.target === d) d.close();
    };
    // The browser keeps the page inert, but lets Tab step out to its own
    // toolbar after the last item; wrap instead, so focus stays in the sheet.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const items = [...d.querySelectorAll<HTMLElement>("a[href], button:not([disabled])")];
      const first = items[0];
      const last = items.at(-1);
      if (!first || !last) return;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    d.addEventListener("close", onClose);
    d.addEventListener("click", onBackdrop);
    d.addEventListener("keydown", onKey);
    // The sheet is for narrow screens: leaving them closes it.
    const mq = window.matchMedia("(min-width: 64rem)");
    const onWide = () => {
      if (mq.matches && d.open) d.close();
    };
    mq.addEventListener("change", onWide);
    return () => {
      d.removeEventListener("close", onClose);
      d.removeEventListener("click", onBackdrop);
      d.removeEventListener("keydown", onKey);
      mq.removeEventListener("change", onWide);
    };
  }, []);

  type NavLink = { href: string; label: string; spy?: string; current?: boolean };
  const links: NavLink[] = [
    ...NAV_ANCHORS.map((l) => ({ href: anchor(l.href), label: nav[l.key], spy: l.href })),
    { href: contact, label: nav.contact, current: page === "contact" },
  ];

  return (
    <header
      ref={headerRef}
      className="fixed inset-x-0 top-0 z-50 border-b border-transparent bg-transparent backdrop-blur-0 transition-[background-color,backdrop-filter,border-color] duration-500 data-[scrolled]:border-[#E8F4FF]/10 data-[scrolled]:bg-[#020514]/55 data-[scrolled]:backdrop-blur-xl"
    >
      {/* O15: rantai-page navbar.tsx's container, logo and wordmark sizes, gaps and pills, 1:1. */}
      <div className="mx-auto flex h-16 w-full max-w-[1440px] items-center justify-between gap-6 px-5 sm:px-8">
        <div className="flex min-w-0 items-center gap-10">
          <a
            href={anchor("#hero")}
            className="flex shrink-0 items-center gap-2.5 rounded-lg text-[#E8F4FF] transition-opacity hover:opacity-75"
          >
            {/* O2: the official mark at its own ratio, sized by height (rantai-page: size-7). */}
            <MascotMark className="h-7 w-auto max-[339px]:h-6" />
            {/* rantai-page's 19 px semibold. "RantAI Lakehouse" is longer than "RantAI", so on the
                narrowest phones it steps down to stay on one line beside the button and the menu. */}
            <span className="text-[19px] font-semibold tracking-[-0.03em] whitespace-nowrap max-[399px]:text-[16px] max-[339px]:text-[14px]">RantAI Lakehouse</span>
          </a>
          <nav aria-label={nav.label} className="hidden lg:block">
            <ul className="flex items-center gap-1">
              {links.map((l) => (
                <li key={l.href}>
                  <a
                    href={l.href}
                    data-spy={page === "home" ? l.spy : undefined}
                    aria-current={l.current ? "page" : undefined}
                    className={pill}
                  >
                    {l.label}
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        </div>
        <div className="flex items-center gap-2">
          <LanguageSwitcher locale={locale} text={language} className="hidden lg:flex" />
          {/* WEB-5 D20: to the page's own "Get started", which links on to the README. */}
          {/* rantai-page's PillLink in the bar: h-10 px-4, h-9 from lg, text-sm. */}
          <CtaLink href={anchor("#get-started")} size="sm" className="h-10 px-4 text-sm lg:h-9">
            {cta.install}
          </CtaLink>
          <button
            ref={buttonRef}
            type="button"
            aria-label={nav.open}
            aria-haspopup="dialog"
            aria-expanded="false"
            aria-controls={sheetId}
            onClick={openSheet}
            className="-mr-2 grid size-10 place-items-center rounded-full text-[#E8F4FF]/85 transition-[background-color,color,transform] duration-200 ease-out hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] active:scale-95 motion-reduce:transition-none lg:hidden"
          >
            <Menu className="size-5" aria-hidden="true" />
          </button>
        </div>
      </div>

      <dialog
        ref={dialogRef}
        id={sheetId}
        aria-labelledby={`${sheetId}-title`}
        aria-describedby={`${sheetId}-desc`}
        // Wheel and touch over the sheet stay with the sheet, not Lenis (D16).
        data-lenis-prevent=""
        // rantai-page's SheetContent (right): w-3/4, max sm, a hairline on its open edge.
        className="lk-sheet fixed inset-y-0 right-0 left-auto m-0 h-[100dvh] max-h-none w-3/4 max-w-none flex-col gap-4 overflow-y-auto border-l border-[#E8F4FF]/10 bg-[#050A30] p-0 text-sm text-[#E8F4FF] shadow-lg backdrop:bg-[#020514]/50 backdrop:backdrop-blur-xs sm:max-w-sm"
      >
        {/* rantai-page's SheetHeader (p-4, gap-0.5, pr-14 beside the close button), SheetTitle and SheetDescription. */}
        <div className="flex flex-col gap-0.5 p-4 pr-14">
          <p id={`${sheetId}-title`} className="text-base font-medium text-[#E8F4FF]">
            {nav.sheetTitle}
          </p>
          <p id={`${sheetId}-desc`} className="text-sm text-[#E8F4FF]/65">
            {nav.sheetDescription}
          </p>
        </div>
        <button
          type="button"
          onClick={closeSheet}
          aria-label={nav.close}
          className="absolute top-2 right-2 grid size-10 place-items-center rounded-full text-[#E8F4FF]/75 transition-[background-color,color,transform] duration-200 ease-out hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] active:scale-90 motion-reduce:transition-none"
        >
          <X className="size-4" aria-hidden="true" />
        </button>
        <nav aria-label={nav.label} className="flex flex-1 flex-col px-4 pb-8">
          <ul>
            {/* The mark is the way home in the bar; the sheet names it too. */}
            {[{ href: anchor("#hero"), label: nav.home } as NavLink, ...links].map((l) => (
              <li key={l.href}>
                <a
                  href={l.href}
                  data-spy={page === "home" ? l.spy : undefined}
                  aria-current={l.current ? "page" : undefined}
                  onClick={closeSheet}
                  className={`group ${sheetLink}`}
                >
                  {l.label}
                  <span
                    aria-hidden="true"
                    className="mr-1 size-1.5 rounded-full bg-[#44A6FA] opacity-0 transition-opacity duration-200 group-aria-[current]:opacity-100"
                  />
                </a>
              </li>
            ))}
          </ul>
          <LanguageSwitcher locale={locale} text={language} size="lg" className="mt-8 self-start" />
        </nav>
      </dialog>
    </header>
  );
}
