/**
 * WEB-5 O23: types the Quickstart terminals (`get-started-play.tsx` loads
 * this chunk when the first terminal comes into view). Terminals type one
 * at a time, in the order they came into view, so two on screen together
 * do not race.
 *
 * Each command's own element keeps its whole text the whole time, split
 * into the typed part and the rest (transparent), so the line wraps exactly
 * as it will and the copy button always copies the whole command. Then the
 * terminal is `done` and CSS brings in its output. Each call returns a stop
 * function that leaves its terminal finished.
 */
// O45: brisk, so all three terminals have typed while the column beside
// them is still pinned (a long URL at 24 ms a character took ~3 s alone).
const CHAR_MS = 14;
const BETWEEN_MS = 180;

let queue: Promise<void> = Promise.resolve();

export function typeTerminal(term: HTMLElement): () => void {
  const lines = [...term.querySelectorAll<HTMLElement>("[data-cmd]")];
  let timer = 0;
  let stopped = false;
  let release: () => void = () => {};

  const finish = () => {
    for (const l of lines) {
      delete l.dataset.typing;
      const real = l.querySelector<HTMLElement>("[data-real]");
      if (real?.dataset.full !== undefined) {
        real.textContent = real.dataset.full;
        delete real.dataset.full;
      }
    }
    term.dataset.term = "done";
    release();
  };

  const typeLine = (i: number) => {
    const line = lines[i];
    const real = line?.querySelector<HTMLElement>("[data-real]");
    if (!line || !real) return finish();
    const text = real.textContent ?? "";
    real.dataset.full = text;
    const typed = document.createElement("span");
    typed.className = "lk-gs-typed";
    const rest = document.createElement("span");
    rest.className = "lk-gs-rest";
    rest.textContent = text;
    real.replaceChildren(typed, rest);
    line.dataset.typing = "";
    let n = 0;
    const step = () => {
      n += 1;
      typed.textContent = text.slice(0, n);
      rest.textContent = text.slice(n);
      if (n < text.length) {
        // A natural pace: a beat longer after a space or a slash.
        const c = text[n - 1];
        timer = window.setTimeout(step, CHAR_MS + (c === " " || c === "/" ? 40 : Math.random() * 12));
        return;
      }
      delete line.dataset.typing;
      line.dataset.typed = "";
      real.textContent = text;
      delete real.dataset.full;
      timer = window.setTimeout(() => typeLine(i + 1), BETWEEN_MS);
    };
    timer = window.setTimeout(step, 120);
  };

  queue = queue.then(
    () =>
      new Promise<void>((resolve) => {
        release = resolve;
        if (stopped) return resolve();
        term.dataset.term = "typing";
        typeLine(0);
      }),
  );

  return () => {
    stopped = true;
    window.clearTimeout(timer);
    finish();
  };
}
