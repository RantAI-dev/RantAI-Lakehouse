"use client";

import * as React from "react";

/**
 * The pace of the answer reveal (`AI-24` D5).
 *
 * The old pacing was 25, 60 or 180 characters a second, by how far behind it
 * was; a hosted model writes about 300 a second, so the reveal fell further
 * behind for a whole answer and the rest arrived in one jump at the end. The
 * pace is now proportional to the backlog: one frame shows the backlog that
 * is 0.3 s of text long, so the reveal trails the received text by about
 * 0.3 s at any model speed, with a 30 characters a second floor so a short
 * answer still moves.
 */

/** How many characters one frame adds: the backlog in 0.3 s, floored at 30/s. */
export function revealStep(behind: number, dtSeconds: number): number {
  if (behind <= 0) return 0;
  return Math.max(1, Math.ceil((behind * dtSeconds) / 0.3), Math.ceil(dtSeconds * 30));
}

/**
 * Reveals `target` a frame at a time instead of in network-sized jumps.
 *
 * The target changes as the answer is written: the draft grows, a new model
 * round starts the draft over, and at `done` the checked answer replaces it.
 * What was already on screen is kept across all three:
 *
 * - a target that starts with the shown text goes on from there, at the same
 *   pace — the checked answer continues the draft instead of appearing whole;
 * - a target that does not (a new round's empty draft, or a checked answer
 *   the citation check rewrote) starts over when it is a fresh draft, and is
 *   shown whole when it is a different answer, as D5 says;
 * - `reduce` (the person's reduced-motion setting) shows everything at once.
 */
export function useSmoothText(target: string, reduce: boolean): string {
  const [reveal, setReveal] = React.useState({ target, shown: 0 });
  let { target: seen, shown } = reveal;

  // A new target is adjusted during render (React's "state from props"
  // pattern) so a replaced answer is never painted one frame of stale text.
  if (seen !== target) {
    const shownText = seen.slice(0, shown);
    const continues = target.length >= shown && target.startsWith(shownText);
    shown = continues ? Math.min(shown, target.length) : target.startsWith(seen) ? 0 : target.length;
    seen = target;
    setReveal({ target: seen, shown });
  }

  // The clock lives across target changes: the target grows on every delta,
  // which restarts the effect, and a per-effect clock would measure a zero
  // frame each time and crawl at one character a frame.
  const lastFrame = React.useRef<number | null>(null);

  React.useEffect(() => {
    if (reduce) return;
    let raf = 0;
    const tick = (now: number) => {
      const previous = lastFrame.current;
      lastFrame.current = now;
      // Cap the frame: a tab that was in the background must not reveal the
      // whole answer in one step when it returns. It catches up at the
      // intended pace over the next few frames instead.
      const dt = previous === null ? 0 : Math.min(0.05, Math.max(0, (now - previous) / 1000));
      setReveal((current) => {
        const behind = target.length - current.shown;
        if (behind <= 0) return current;
        return {
          target,
          shown: Math.min(target.length, current.shown + revealStep(behind, dt)),
        };
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target, reduce]);

  if (reduce) return target;
  return target.slice(0, Math.min(shown, target.length));
}
