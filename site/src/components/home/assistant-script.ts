import { S } from "./assistant-steps";

/**
 * WEB-5 O19: the assistant panel's script, loaded as its own chunk when the
 * panel comes into view (`assistant-play.tsx`), so none of it counts
 * against the initial-JS budget. It adds one step at a time to the panel's
 * `data-s`; CSS draws each step. Typing writes the composer's text
 * directly; the two questions are read from the panel's own bubbles, so the
 * words stay in the server-rendered messages (D3). Returns a stop function
 * that leaves the panel finished.
 */

type Step = (typeof S)[keyof typeof S];

/** Milliseconds to wait before each step; the two typing steps add their own typing time. */
const SCRIPT: readonly (readonly [Step, number])[] = [
  [S.type1, 450],
  [S.ask1, 350],
  [S.think1a, 450],
  [S.think1b, 900],
  [S.answer, 900],
  [S.build, 2300],
  [S.type2, 450],
  [S.ask2, 350],
  [S.think2, 450],
  [S.card, 1000],
  [S.press, 2800],
  [S.approved, 260],
];
const TYPE_MS = 30;

export type Player = { stop: () => void; pause: () => void; resume: () => void };

/**
 * Plays the script. `pause` and `resume` hold and continue it where it was (the visitor scrolled
 * back up and the panel is no longer in view): a step waiting for its turn waits again, and a
 * question half typed carries on from the letter it reached.
 */
export function play(panel: HTMLElement, from = 0): Player {
  const typed = panel.querySelector<HTMLElement>("[data-typed]");
  const questions = [...panel.querySelectorAll<HTMLElement>("[data-q]")].map((q) => q.textContent?.trim() ?? "");
  // WEB-5 M1: a phone starts with the first exchange already shown (`FIRST_EXCHANGE` in
  // assistant-steps.ts), so the script carries on from the step after it. The script's first
  // steps are the steps in their own order, so `from` steps reached are `SCRIPT[0..from)`.
  const reached: Step[] = SCRIPT.slice(0, from).map(([s]) => s);
  let timer = 0;
  let typing = 0;
  let paused = false;
  let again: (() => void) | null = null;

  const enter = (step: Step) => {
    reached.push(step);
    panel.dataset.s = reached.join(" ");
  };
  const type = (text: string, from: number, done: () => void) => {
    let n = from;
    again = () => type(text, n, done);
    typing = window.setInterval(() => {
      n += 1;
      if (typed) typed.textContent = text.slice(0, n);
      if (n >= text.length) {
        window.clearInterval(typing);
        again = null;
        done();
      }
    }, TYPE_MS);
  };
  const run = (i: number) => {
    if (i >= SCRIPT.length) {
      again = null;
      return;
    }
    const [step, wait] = SCRIPT[i];
    again = () => run(i);
    timer = window.setTimeout(() => {
      enter(step);
      if (step === S.type1 || step === S.type2) {
        type(questions[step === S.type1 ? 0 : 1] ?? "", 0, () => run(i + 1));
        return;
      }
      if ((step === S.ask1 || step === S.ask2) && typed) typed.textContent = "";
      run(i + 1);
    }, wait);
  };
  run(from);

  return {
    stop: () => {
      window.clearTimeout(timer);
      window.clearInterval(typing);
      if (typed) typed.textContent = "";
      panel.dataset.s = SCRIPT.map(([s]) => s).join(" ");
    },
    pause: () => {
      if (paused || !again) return;
      paused = true;
      window.clearTimeout(timer);
      window.clearInterval(typing);
    },
    resume: () => {
      if (!paused) return;
      paused = false;
      again?.();
    },
  };
}
