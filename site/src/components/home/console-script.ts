import { CONSOLE_PROMPTS } from "./console-prompts";

/**
 * The console-home picture's script (loaded as its own chunk by `console-play.tsx`): a visitor
 * using the console, in two scenes of about 12 s, looping.
 *  A. type "Which tables ch", a greyed completion appears, Tab presses (120 ms) and snaps it in;
 *     send; Ask slides to Build and the placeholder turns into a build prompt; a ghost pointer
 *     moves to "Build a dashboard", the card lifts, presses (scale .98, a ripple), and the composer
 *     morphs (FLIP) into a small dashboard that assembles tile by tile; hold; morph back.
 *  B. the same with "What data do we h", Build back to Ask, "Connect a source", and a list of
 *     sources that tick from Connecting to Connected.
 * `setActive(false)` holds it where it is (hidden tab, off screen, hovered); a click on a shortcut
 * plays its result at once and then the loop carries on. Timings of the small states are 120 to
 * 250 ms with soft easings; the morph is 420 ms. No dependency: the Web Animations API.
 */
type Kind = "dash" | "src";
class Cancel extends Error {}

export function start(root: HTMLElement): { setActive: (on: boolean) => void; stop: () => void } {
  const $ = <T extends HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const typed = $("[data-typed]");
  const ghost = $("[data-ghost]");
  const tab = $(".lk-con-tab");
  const stage = $(".lk-con-stage");
  const box = $(".lk-con-box");
  const result = $(".lk-con-result");
  const pointer = $(".lk-con-pointer");
  const cards: Record<string, HTMLElement> = Object.fromEntries([...root.querySelectorAll<HTMLElement>("[data-sc]")].map((c) => [c.dataset.sc ?? "", c]));
  if (!typed || !ghost || !stage || !box || !result || !pointer) return { setActive: () => {}, stop: () => {} };

  let active = false;
  let epoch = 0;
  let takeover: Promise<void> = Promise.resolve();
  let stopped = false;
  const myEpoch = () => epoch;

  /** Waits `ms` of active time; throws Cancel if the epoch moved on (a click took over) or it was stopped. */
  const wait = async (ms: number, e: number) => {
    let left = ms;
    while (left > 0) {
      if (stopped || e !== epoch) throw new Cancel();
      await new Promise((r) => setTimeout(r, 40));
      if (active) left -= 40;
    }
    if (stopped || e !== epoch) throw new Cancel();
  };

  const setText = (full: string, n: number) => {
    typed.textContent = full.slice(0, n);
    ghost.textContent = full.slice(n);
  };
  const type = async (full: string, upTo: number, e: number) => {
    ghost.textContent = "";
    for (let n = 1; n <= upTo; n++) {
      typed.textContent = full.slice(0, n);
      await wait(34, e);
    }
  };
  const mode = (m: "ask" | "build") => {
    root.dataset.mode = m;
  };
  const movePointerTo = (el: HTMLElement | null) => {
    if (!el) return;
    const r = root.getBoundingClientRect();
    const c = el.getBoundingClientRect();
    pointer.style.opacity = "1";
    pointer.style.transform = `translate(${c.left - r.left + c.width * 0.62}px, ${c.top - r.top + c.height * 0.58}px)`;
  };
  const press = async (el: HTMLElement, e: number) => {
    const c = el.getBoundingClientRect();
    el.style.setProperty("--rx", "62%");
    el.style.setProperty("--ry", `${(c.height * 0.58 / c.height) * 100}%`);
    el.setAttribute("data-hover", "");
    await wait(450, e);
    el.setAttribute("data-press", "");
    await wait(170, e);
    el.removeAttribute("data-press");
  };
  const clear = (el: HTMLElement | undefined) => {
    el?.removeAttribute("data-hover");
    el?.removeAttribute("data-press");
  };

  const morph = (from: HTMLElement, to: HTMLElement, back: boolean) => {
    const a = from.getBoundingClientRect();
    const b = to.getBoundingClientRect();
    const sx = a.width / b.width;
    const sy = a.height / b.height;
    const frame = { transform: `translate(${a.left - b.left}px, ${a.top - b.top}px) scale(${sx}, ${sy})`, borderRadius: "16px" };
    const rest = { transform: "none", borderRadius: "16px" };
    return to.animate(back ? [rest, frame] : [frame, rest], { duration: 420, easing: "cubic-bezier(0.16, 1, 0.3, 1)", fill: "both" }).finished;
  };

  const count = (el: HTMLElement | null) => {
    if (!el) return;
    const target = Number(el.dataset.count ?? 0);
    const t0 = performance.now();
    const tick = (t: number) => {
      const p = Math.min(1, (t - t0) / 900);
      el.textContent = `${(target * (1 - Math.pow(1 - p, 3))).toFixed(1)}k`;
      if (p < 1 && !stopped) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };
  const chips = () => [...result.querySelectorAll<HTMLElement>(".lk-cv-chip")];

  const showResult = async (kind: Kind, hold: number, e: number) => {
    result.dataset.kind = kind;
    result.removeAttribute("data-show");
    stage.setAttribute("data-result", "");
    if (kind === "src") chips().forEach((c) => (c.dataset.state = "connecting"));
    const m = morph(box, result, false);
    await wait(60, e);
    result.setAttribute("data-show", "");
    void m;
    if (kind === "dash") {
      await wait(300, e);
      count(result.querySelector<HTMLElement>("[data-count]"));
    } else {
      await wait(900, e);
      for (const c of chips()) {
        c.dataset.state = "connected";
        await wait(380, e);
      }
    }
    await wait(hold, e);
    await closeResult();
  };
  const closeResult = async () => {
    if (!stage.hasAttribute("data-result")) return;
    result.removeAttribute("data-show");
    await morph(box, result, true).catch(() => {});
    stage.removeAttribute("data-result");
    result.getAnimations().forEach((a) => a.cancel());
    result.dataset.kind = "";
  };
  const reset = () => {
    stage.removeAttribute("data-result");
    result.getAnimations().forEach((a) => a.cancel());
    result.removeAttribute("data-show");
    result.dataset.kind = "";
    Object.values(cards).forEach(clear);
    pointer.style.opacity = "0";
    tab?.removeAttribute("data-pressed");
  };

  const scene = async (kind: Kind, e: number) => {
    const ask = kind === "dash" ? CONSOLE_PROMPTS.ask : CONSOLE_PROMPTS.ask2;
    mode("ask");
    setText(ask, 0);
    await wait(500, e);
    const part = kind === "dash" ? 15 : 17;
    await type(ask, part, e);
    setText(ask, part); // the rest shows greyed
    await wait(600, e);
    tab?.setAttribute("data-pressed", "");
    await wait(130, e);
    tab?.removeAttribute("data-pressed");
    setText(ask, ask.length);
    await wait(900, e);
    typed.textContent = "";
    ghost.textContent = "";
    if (kind === "dash") {
      mode("build");
      ghost.textContent = CONSOLE_PROMPTS.build;
      await wait(900, e);
    } else {
      mode("ask");
      ghost.textContent = CONSOLE_PROMPTS.ask;
      await wait(500, e);
    }
    const card = cards[kind];
    movePointerTo(card ?? null);
    await wait(800, e);
    if (card) await press(card, e);
    pointer.style.opacity = "0";
    await showResult(kind, kind === "dash" ? 4200 : 3000, e);
    if (card) clear(card);
    await wait(500, e);
  };

  const loop = async () => {
    let n = 0;
    while (!stopped) {
      const e = myEpoch();
      try {
        await scene(n % 2 === 0 ? "dash" : "src", e);
        n += 1;
      } catch (err) {
        if (!(err instanceof Cancel)) throw err;
        if (stopped) return;
        // A click took over: it plays its own result (below) and the loop picks up after it.
        await takeover;
      }
    }
  };

  const onClick = (ev: MouseEvent) => {
    const card = (ev.target as HTMLElement).closest<HTMLElement>("[data-sc]");
    const id = card?.dataset.sc;
    if (id !== "dash" && id !== "src") return;
    epoch += 1;
    reset();
    const e = myEpoch();
    // Hold while the visitor's own result is shown; the loop's scene was cancelled and restarts after.
    takeover = (async () => {
      try {
        setText(id === "dash" ? CONSOLE_PROMPTS.ask : CONSOLE_PROMPTS.ask2, 0);
        mode(id === "dash" ? "build" : "ask");
        await showResult(id, 5000, e);
      } catch {
        /* superseded by another click */
      }
    })();
  };
  root.addEventListener("click", onClick);
  void loop();

  return {
    setActive: (on) => {
      active = on;
    },
    stop: () => {
      stopped = true;
      root.removeEventListener("click", onClick);
      reset();
    },
  };
}
