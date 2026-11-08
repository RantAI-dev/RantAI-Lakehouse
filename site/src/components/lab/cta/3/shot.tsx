"use client";

import * as React from "react";

/**
 * The console window, easing up a little as the card enters the view (once; a still picture
 * under reduced motion and without JavaScript). Framed by one hairline, with no browser chrome.
 */
export function Shot() {
  const ref = React.useRef<HTMLDivElement>(null);
  const [up, setUp] = React.useState(false);
  React.useEffect(() => {
    const el = ref.current;
    if (!el || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      setUp(true);
      return;
    }
    const io = new IntersectionObserver(
      ([e]) => {
        if (e.isIntersecting) {
          setUp(true);
          io.disconnect();
        }
      },
      { threshold: 0.25 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return (
    <div ref={ref} className="relative h-full overflow-hidden rounded-b-[28px] max-md:h-[240px] md:rounded-br-[28px] md:rounded-bl-none">
      <div
        data-up={up ? "" : undefined}
        className="absolute inset-x-6 top-8 translate-y-10 overflow-hidden rounded-t-2xl bg-white ring-1 ring-white/15 transition-transform duration-[900ms] ease-[cubic-bezier(0.16,1,0.3,1)] data-[up]:translate-y-0 motion-reduce:transition-none md:inset-x-8 md:top-12 md:left-4"
      >
        {/* eslint-disable-next-line @next/next/no-img-element -- a static export has no image optimizer; the file is already at its displayed size */}
        <img
          src="/products/lake-dashboard.webp"
          width={1200}
          height={704}
          loading="lazy"
          decoding="async"
          alt="The Lakehouse console's Dashboards page: four number cards, a visitor trend chart and two bar and ring charts, with illustrative data."
          className="block h-auto w-full"
        />
      </div>
    </div>
  );
}
