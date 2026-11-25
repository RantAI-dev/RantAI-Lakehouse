"use client";

import * as React from "react";
import { afterPaint } from "@/lib/after-paint";
import { QuietBoundary } from "./quiet-boundary";

const StillContext = React.createContext<React.ReactNode>(null);

/** The still picture again, for a `next/dynamic` `loading` option: the page keeps what it shows until the live one arrives. */
export function StillSlot() {
  return <>{React.useContext(StillContext)}</>;
}

/**
 * A picture that is still in the page's first JavaScript and live later
 * (WEB-5 T3, the size budget of D5).
 *
 * The server renders the picture once, from a stateless view, and hands it in
 * as `children`: that is what the page shows, with or without JavaScript.
 * After the first paint, and only while the picture is off screen, the live
 * component (a `next/dynamic` chunk with `ssr: false`, so the HTML does not
 * list it) takes its place. It renders the same elements from the same view,
 * so nothing moves or flickers, and adds the loop and the pointer. If the
 * chunk never loads, the still picture stays. `loading` of the live
 * component must be `StillSlot`.
 */
export function DeferredLive<P extends object>({
  live: Live,
  liveProps,
  children,
}: Readonly<{ live: React.ComponentType<P>; liveProps: P; children: React.ReactNode }>) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [wanted, setWanted] = React.useState(false);
  const [failed, setFailed] = React.useState(false);
  React.useEffect(() => {
    let io: IntersectionObserver | null = null;
    const cancel = afterPaint(() => {
      const el = ref.current?.firstElementChild;
      if (!el) return setWanted(true);
      io = new IntersectionObserver(([e]) => {
        if (e.isIntersecting) return;
        io?.disconnect();
        setWanted(true);
      });
      io.observe(el);
    });
    return () => {
      cancel();
      io?.disconnect();
    };
  }, []);
  return (
    <div ref={ref} className="contents">
      <StillContext.Provider value={children}>
        {wanted && !failed ? (
          <QuietBoundary onError={() => setFailed(true)}>
            <Live {...liveProps} />
          </QuietBoundary>
        ) : (
          children
        )}
      </StillContext.Provider>
    </div>
  );
}
