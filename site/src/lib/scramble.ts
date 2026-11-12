/**
 * Text scrambling for the masking demo (S6). Pure, so the rules are tested:
 * what masked values look like, and what a frame of the transition may show.
 */

const GLYPHS = "abcdefghijklmnopqrstuvwxyz0123456789#$%&*+=?";
const DOT = "•";

/** `Mira Okafor` -> `M••• O•••`: the initial of each word, then dots. */
export function maskName(name: string): string {
  return name
    .split(" ")
    .map((w) => (w ? `${w[0]}${DOT.repeat(3)}` : w))
    .join(" ");
}

/** `mira.okafor@example.com` -> `m•••@•••.com`: first letter, then dots, then the top-level domain. */
export function maskEmail(email: string): string {
  const at = email.indexOf("@");
  if (at < 1) return DOT.repeat(3);
  const tld = email.slice(email.lastIndexOf(".") + 1);
  return `${email[0]}${DOT.repeat(3)}@${DOT.repeat(3)}.${tld}`;
}

const isKept = (c: string) => c === " " || c === "@" || c === "." || c === DOT;

/**
 * One frame of the cycle from `from` to `to`, `progress` in 0..1.
 *
 * At 0 it is exactly `from` and at 1 exactly `to`. In between, the length
 * eases from one to the other, characters left of the moving front show the
 * final value, and the rest cycle through random glyphs, except spaces, `@`,
 * `.` and the mask dots, which keep their place so the shape stays readable.
 * `rand` is injected so tests are deterministic.
 */
export function scrambleFrame(from: string, to: string, progress: number, rand: () => number): string {
  if (progress <= 0) return from;
  if (progress >= 1) return to;
  const len = Math.max(1, Math.round(from.length + (to.length - from.length) * progress));
  const front = progress * len;
  let out = "";
  for (let i = 0; i < len; i++) {
    const target = to[i] ?? from[i] ?? "";
    if (i < front) out += target;
    else if (isKept(target)) out += target;
    else out += GLYPHS[Math.floor(rand() * GLYPHS.length)];
  }
  return out;
}
