/**
 * The whale's head for lab 2: the mascot's navy dome (the same proportions
 * as `shared/whale-svg.tsx`'s body above its waterline) with its two plain
 * white eyes, no glow (O29, O48). Only the head: it peeks out from behind a
 * letter. Drawn in 400 x 200 units, the dome's base on the bottom edge.
 */
export function Lab2Whale({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 400 200" className={className} aria-hidden="true" focusable="false">
      <path d="M8 200 A192 184 0 0 1 392 200 Z" fill="#0b1658" />
      <path d="M60 70 A192 184 0 0 1 300 22" fill="none" stroke="#44a6fa" strokeOpacity="0.45" strokeWidth="6" strokeLinecap="round" />
      <g data-eyes>
        <circle cx="132" cy="112" r="26" fill="#ffffff" />
        <circle cx="268" cy="112" r="26" fill="#ffffff" />
      </g>
    </svg>
  );
}
