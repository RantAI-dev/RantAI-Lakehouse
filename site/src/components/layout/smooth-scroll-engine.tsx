"use client";

import * as React from "react";
import Lenis from "lenis";
import "lenis/dist/lenis.css";
import { SCROLL_TOP_EVENT } from "./footer-motion";

/** Keys the browser scrolls with; pressing one hands the page back to native scrolling at once. */
const SCROLL_KEYS = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ", "Spacebar"]);

/**
 * Lenis itself (WEB-5 D16), loaded only by `smooth-scroll.tsx`.
 *
 * Why it agrees with the scroll story (D14). Lenis is not a virtual
 * scroller: it eases the wheel and then moves the real window, so every
 * scroll-driven piece of the page (the depth gauge, the water, the
 * sections' progress, the 3D engine) keeps reading `window.scrollY` from
 * native `scroll` events and sees the same position for the same scroll,
 * with or without it (measured at the chapter marks in the P2 evidence).
 *
 * What it changes, and why:
 * - Only the wheel. Keyboard scrolling stays native, and a scroll key
 *   stops an easing in progress first, so the two never pull against each
 *   other. Touch is not smoothed (`syncTouch` off).
 * - In-page links (`#sources`, the skip link `#main`) are taken here, not
 *   through Lenis's own `anchors` option, which smooth-scrolls but lets the
 *   browser's hash jump run as well. This handler cancels the jump, eases
 *   to the target (Lenis honours the sections' `scroll-margin-top`, so they
 *   stay clear of the top bar), writes the hash, and moves keyboard focus
 *   to the target as the browser would, which the skip link relies on.
 * - `allowNestedScroll`: a scrollable box under the pointer (the message
 *   field, a wide table) scrolls itself instead of the page.
 */
export default function SmoothScrollEngine() {
  React.useEffect(() => {
    const lenis = new Lenis({ lerp: 0.1, autoRaf: true, allowNestedScroll: true });

    const onClick = (event: MouseEvent) => {
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      const link = (event.target as Element | null)?.closest?.("a[href]");
      if (!(link instanceof HTMLAnchorElement) || (link.target && link.target !== "_self")) return;
      const url = new URL(link.href);
      if (url.origin !== window.location.origin || url.pathname !== window.location.pathname || !url.hash) return;
      const target = document.getElementById(decodeURIComponent(url.hash.slice(1)));
      if (!target) return;
      event.preventDefault();
      lenis.scrollTo(target);
      if (window.location.hash !== url.hash) history.pushState(null, "", url.hash);
      // Move focus as the browser's own jump does: a target that cannot take
      // focus gets a temporary tabindex, so the next Tab starts from it.
      if (!target.hasAttribute("tabindex")) {
        target.setAttribute("tabindex", "-1");
        target.addEventListener("blur", () => target.removeAttribute("tabindex"), { once: true });
      }
      target.focus({ preventScroll: true });
    };
    const onKey = (event: KeyboardEvent) => {
      // `reset()` is private in Lenis's types; `stop()` then `start()` is its
      // public form (each calls it), and run back to back the stopped state
      // never reaches a layout, so the key's native scroll is not blocked.
      if (SCROLL_KEYS.has(event.key) && lenis.isScrolling === "smooth") {
        lenis.stop();
        lenis.start();
      }
    };

    // The footer's Back to top (O7) asks for a glide through this event.
    const onTop = () => lenis.scrollTo(0);
    document.addEventListener("click", onClick);
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener(SCROLL_TOP_EVENT, onTop);
    return () => {
      window.removeEventListener(SCROLL_TOP_EVENT, onTop);
      document.removeEventListener("click", onClick);
      window.removeEventListener("keydown", onKey, { capture: true });
      lenis.destroy();
    };
  }, []);

  return null;
}
