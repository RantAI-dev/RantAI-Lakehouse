/**
 * WEB-5 O44/O45 guard: a layout probe for the two sticky sections the owner
 * caught broken in a real browser, run against a served build in headless
 * Chrome. Layout cannot be checked in `bun test` (no browser), so this is a
 * script a developer or reviewer runs before handing off a change to them:
 *
 *   google-chrome --headless=new --remote-debugging-port=3429 &
 *   python3 -m http.server 3421 --directory out &
 *   CDP_PORT=3429 bun scripts/probe-layout.ts http://localhost:3421/
 *
 * For each viewport (MacBook inner sizes, the owner's VS Code browser at
 * 1190 px, a sweep of 1100-1279 px and of heights 500-900) it scrolls
 * through the page and fails (exit 1) when:
 * - "Yours to keep": two showcase list items ever overlap (O44: at the
 *   release all four were pushed into one spot), or more than one picture
 *   is shown at once.
 * It also prints how long "Get started"'s left column stays pinned, for
 * information only: since the O45 rollback the steps keep their compact
 * spacing and the column pins only as long as they naturally take.
 */

export {};

const PORT = Number(process.env.CDP_PORT ?? 9222);
const base = process.argv[2] ?? "http://localhost:3421/";

const SIZES: [number, number][] = [
  [1512, 860], [1470, 830], [1440, 780], [1280, 720], [1190, 960], [1190, 700],
  ...[1100, 1150, 1200, 1250, 1279].map((w) => [w, 900] as [number, number]),
  ...[500, 550, 600, 650, 700, 750, 800, 850, 900].map((h) => [1280, h] as [number, number]),
];

type Cdp = { ev: (expr: string) => Promise<unknown>; close: () => void };
type Reply = { id?: number; result?: { result?: { value?: unknown } } };

async function open(w: number, h: number, url = base): Promise<Cdp> {
  const t = await (await fetch(`http://127.0.0.1:${PORT}/json/new`, { method: "PUT" })).json();
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let id = 0;
  const pending = new Map<number, (v: Reply) => void>();
  ws.onmessage = (e) => {
    const m = JSON.parse(String(e.data)) as Reply;
    const done = m.id ? pending.get(m.id) : undefined;
    if (done && m.id) {
      pending.delete(m.id);
      done(m);
    }
  };
  const send = (method: string, params: object = {}) =>
    new Promise<Reply>((r) => {
      const i = ++id;
      pending.set(i, r);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  await send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: w < 600 });
  await send("Page.enable");
  await send("Page.navigate", { url });
  const ev = async (expression: string) =>
    (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result?.result?.value;
  for (let i = 0; i < 200 && (await ev("document.readyState")) !== "complete"; i++) await Bun.sleep(100);
  await Bun.sleep(1200);
  await ev("document.documentElement.style.scrollBehavior='auto'");
  return { ev, close: () => ws.close() };
}

const PROBE = `(async () => {
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const out = { overlap: 0, shown: 0, pinned: 0 };
  const show = document.querySelector('#open');
  const top = show.getBoundingClientRect().top + scrollY;
  for (let y = top - innerHeight; y < top + show.offsetHeight; y += 40) {
    scrollTo(0, y); await frame();
    const items = [...document.querySelectorAll('.lk-show-item')].map((e) => e.getBoundingClientRect()).filter((r) => r.height > 0);
    for (let i = 0; i < items.length; i++) for (let j = i + 1; j < items.length; j++) {
      const a = items[i], b = items[j];
      const v = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top), hz = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      if (v > 1 && hz > 1) out.overlap = Math.max(out.overlap, Math.round(v));
    }
    const visible = [...document.querySelectorAll('.lk-show-visual')].filter((e) => getComputedStyle(e).visibility === 'visible' && +getComputedStyle(e).opacity > 0.5);
    const stacked = getComputedStyle(document.querySelector('.lk-show-stage')).display !== 'contents';
    if (stacked) out.shown = Math.max(out.shown, visible.length);
  }
  const gs = document.querySelector('#get-started'), col = gs.querySelector('.lk-sticky-center-md');
  const gtop = gs.getBoundingClientRect().top + scrollY;
  let first = null, last = null;
  for (let y = gtop - 200; y < gtop + gs.offsetHeight; y += 40) {
    scrollTo(0, y); await frame();
    const t = col.getBoundingClientRect().top;
    if (Math.abs(t - parseFloat(getComputedStyle(col).top)) <= 1 && getComputedStyle(col).position === 'sticky') { first ??= y; last = y; }
  }
  out.pinned = first === null ? 0 : last - first;
  return out;
})()`;

/**
 * Owner QA (the depths stage, 3D path): from md up, the active depth's heading and line sit directly
 * under its rail label (`.lk-depth-rail`), on the label's text edge, and never touch another label.
 * For each width, language and stage (Bronze, Silver, Gold: the pinned section at 1/6, 1/2 and
 * 5/6 of its scroll) it measures, after the 350 ms swap has settled:
 * - the block's left x against the active label text's (within 2 px) and its top below the label's bottom;
 * - no intersection with any other label row;
 * - the heading and the line stay inside the window (bottom clear of the edge by BOTTOM px, for the language toast).
 * On phones the rail is the bare line and the heading stays in the flow: the rows (`.lk-depth-rail-m > div`)
 * must only not intersect the heading or its line.
 */
const RAIL_PROBE = (md: boolean, bottom: number) => `(async () => {
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const sec = document.querySelector('#depths');
  const top = sec.getBoundingClientRect().top + scrollY;
  const res = [];
  for (const k of [1 / 6, 1 / 2, 5 / 6]) {
    scrollTo(0, top + k * (sec.offsetHeight - innerHeight)); await frame(); await new Promise((r) => setTimeout(r, 1200));
    const errs = [];
    if (${md}) {
      const items = [...document.querySelectorAll('.lk-depth-rail > div')];
      const act = items.find((e) => e.querySelector('.lk-depth-block[aria-live]'));
      const lab = act.querySelector('span.uppercase').getBoundingClientRect();
      const h = act.querySelector('h3').getBoundingClientRect(), p = act.querySelector('h3 + p').getBoundingClientRect();
      if (Math.abs(h.left - lab.left) > 2) errs.push('left ' + Math.round(h.left - lab.left));
      if (h.top < lab.bottom) errs.push('heading above label');
      for (const it of items) if (it !== act) {
        const r = it.querySelector('span.uppercase').getBoundingClientRect();
        for (const b of [h, p]) if (Math.min(r.bottom, b.bottom) - Math.max(r.top, b.top) > 0 && Math.min(r.right, b.right) - Math.max(r.left, b.left) > 0) errs.push('hits a label');
        if (r.top < p.bottom && r.bottom > h.top) errs.push('label beside block');
      }
      if (h.top < 64 || p.bottom > innerHeight - ${bottom}) errs.push('outside window ' + Math.round(h.top) + '..' + Math.round(p.bottom) + '/' + innerHeight);
      res.push(errs.length ? errs.join(',') : 'ok(+' + Math.round(h.top - lab.bottom) + ')');
    } else {
      const rows = [...document.querySelectorAll('.lk-depth-rail-m > div')].map((e) => e.getBoundingClientRect());
      const text = [...document.querySelectorAll('.lk-depth-grid-stage h3, .lk-depth-grid-stage h3 + p')].map((e) => e.getBoundingClientRect()).filter((r) => r.width > 0);
      let hit = false;
      for (const a of rows) for (const b of text) if (Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 0 && Math.min(a.right, b.right) - Math.max(a.left, b.left) > 0) hit = true;
      res.push(hit ? 'hit' : 'ok');
    }
  }
  return res;
})()`;

/**
 * Owner QA, the assistant: (1) the chat must not have played before the visitor can see it: at the
 * first scroll position where the depths stage has (nearly) left: its section's bottom is above 35 % of the screen, `data-s` is still
 * the start (at most the first two steps), and the panel is not finished; (2) the depths rail is
 * gone (opacity 0, or out of view) once the assistant's heading is on screen.
 */
const ASSISTANT_PROBE = `(async () => {
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const depths = document.querySelector('#depths'), panel = document.querySelector('#assistant .lk-ast'), head = document.querySelector('#assistant h2');
  const rail = document.querySelector('.lk-depth-rail');
  const y0 = depths.getBoundingClientRect().top + scrollY + depths.offsetHeight - innerHeight * 1.2;
  const out = { startSteps: -1, railWorst: 0 };
  for (let y = y0; y < y0 + innerHeight * 2; y += 25) {
    scrollTo(0, y); await frame();
    const p = panel.getBoundingClientRect(), d = depths.getBoundingClientRect(), h = head.getBoundingClientRect();
    if (out.startSteps < 0 && d.bottom <= innerHeight * 0.35) out.startSteps = (panel.dataset.s || '').split(' ').filter(Boolean).length;
    const r = rail.getBoundingClientRect();
    if (h.top < innerHeight && h.bottom > 0 && r.bottom > 0 && r.top < innerHeight) out.railWorst = Math.max(out.railWorst, +getComputedStyle(rail).opacity);
  }
  return out;
})()`;
/**
 * Owner QA, "Yours to keep": while the showcase is pinned (at its first and its third promise) the
 * heading is fully on screen and below the top bar, it is at most 96 px above the first item, and
 * the list and the panel are not clipped by the bottom of the window.
 */
const SHOW_PROBE = `(async () => {
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const out = { pinned: false, headTop: 0, headBottom: 0, gap: 0, bottom: 0, vh: innerHeight };
  const marks = [...document.querySelectorAll('.lk-show-mark')];
  if (getComputedStyle(marks[0]).display === 'none') return out;
  out.pinned = true;
  for (const i of [0, 2]) {
    const m = marks[i].getBoundingClientRect();
    scrollTo(0, scrollY + m.top + m.height / 2 - innerHeight / 2); await frame(); await new Promise((r) => setTimeout(r, 700));
    const h = document.querySelector('.lk-show-head h2').getBoundingClientRect();
    const first = document.querySelector('.lk-show-item').getBoundingClientRect();
    const list = document.querySelector('.lk-show-list').getBoundingClientRect();
    const vis = document.querySelector('.lk-show-visual[data-on]').getBoundingClientRect();
    out.headTop = out.headTop ? Math.min(out.headTop, h.top) : h.top;
    out.headBottom = Math.max(out.headBottom, h.bottom);
    out.gap = Math.max(out.gap, first.top - h.bottom);
    out.bottom = Math.max(out.bottom, list.bottom, vis.bottom);
  }
  return out;
})()`;
/**
 * Owner QA, "Guarded by default": on desktop the table and the whole bento grid fit in one window
 * together: from the table's top to the grid's bottom is at most the window height minus the top
 * bar (64 px) and 32 px of air. Also, the audit tile shows one status at a time (never both pills).
 */
const GOV_PROBE = `(async () => {
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const card = document.querySelector('#governance table').closest('[class*="rounded-3xl"]');
  scrollTo(0, card.getBoundingClientRect().top + scrollY - 80); await frame(); await new Promise((r) => setTimeout(r, 1200));
  const t = card.getBoundingClientRect(), u = document.querySelector('#governance .lk-mb ul').getBoundingClientRect();
  let both = 0;
  const wait = document.querySelector('.lk-gv-wait'), ok = document.querySelector('.lk-gv-ok');
  for (let i = 0; i < 140; i++) {
    const a = +getComputedStyle(wait).opacity, b = +getComputedStyle(ok).opacity;
    if (a > 0.02 && b > 0.02) both++;
    await new Promise((r) => setTimeout(r, 100));
  }
  return { span: u.bottom - t.top, budget: innerHeight - 64 - 32, both };
})()`;
let failed = 0;
for (const [w, h] of [[1190, 800], [1280, 800], [1440, 900]]) {
  const p = await open(w, h);
  const r = (await p.ev(GOV_PROBE)) as { span: number; budget: number; both: number };
  p.close();
  const errors: string[] = [];
  if (r.span > r.budget) errors.push(`table + bento is ${Math.round(r.span)}px, over the ${r.budget}px that fit`);
  if (r.both > 0) errors.push(`both audit statuses visible together in ${r.both} samples`);
  console.log(`governance ${w}x${h}`, errors.length ? `FAIL: ${errors.join("; ")}` : `ok (${Math.round(r.span)}/${r.budget}px)`);
  if (errors.length) failed++;
}
for (const [w, h] of [[1190, 960], [1150, 810], [1440, 900]]) {
  const p = await open(w, h);
  const r = (await p.ev(SHOW_PROBE)) as { pinned: boolean; headTop: number; headBottom: number; gap: number; bottom: number; vh: number };
  p.close();
  const errors: string[] = [];
  if (!r.pinned) errors.push("the showcase is not pinned at this size");
  else {
    if (r.headTop < 64) errors.push(`the heading is under the top bar (top ${Math.round(r.headTop)}px)`);
    if (r.headBottom > r.vh) errors.push("the heading is below the window");
    if (r.gap > 96 || r.gap < 0) errors.push(`heading to first item gap ${Math.round(r.gap)}px (want 0 to 96)`);
    if (r.bottom > r.vh) errors.push(`the unit is clipped by ${Math.round(r.bottom - r.vh)}px`);
  }
  console.log(`showcase ${w}x${h}`, errors.length ? `FAIL: ${errors.join("; ")}` : `ok (head top ${Math.round(r.headTop)}, gap ${Math.round(r.gap)}, bottom ${Math.round(r.bottom)}/${r.vh})`);
  if (errors.length) failed++;
}
{
  const p = await open(1190, 960);
  const r = (await p.ev(ASSISTANT_PROBE)) as { startSteps: number; railWorst: number };
  p.close();
  const errors: string[] = [];
  if (r.startSteps > 2) errors.push(`the chat had already played ${r.startSteps} steps when it first came uncovered`);
  if (r.railWorst > 0.02) errors.push(`the depths rail shows (opacity ${r.railWorst}) over the assistant`);
  console.log("assistant 1190x960", errors.length ? `FAIL: ${errors.join("; ")}` : `ok (steps at first sight ${r.startSteps}, rail opacity ${r.railWorst})`);
  if (errors.length) failed++;
}
for (const [w, h] of [[390, 844], [1024, 800], [1190, 735], [1190, 960], [1280, 800], [1440, 900]]) {
  for (const lang of ["", "id/"]) {
    const url = new URL(lang, base).href;
    const p = await open(w, h, url);
    const r = (await p.ev(RAIL_PROBE(w >= 768, 24))) as string[];
    p.close();
    const bad = r.some((x) => !x.startsWith("ok"));
    console.log(`depths ${w}x${h} ${lang || "en/"} (bronze, silver, gold)`, bad ? `FAIL: ${r.join(" | ")}` : r.join(" "));
    if (bad) failed++;
  }
}
for (const [w, h] of SIZES) {
  const p = await open(w, h);
  const r = (await p.ev(PROBE)) as { overlap: number; shown: number; pinned: number };
  p.close();
  const errors: string[] = [];
  if (r.overlap > 0) errors.push(`showcase items overlap by ${r.overlap}px`);
  if (r.shown > 1) errors.push(`${r.shown} showcase pictures shown at once`);
  console.log(`${w}x${h}`, errors.length ? `FAIL: ${errors.join("; ")}` : `ok (Get started pinned ${r.pinned}px)`);
  if (errors.length) failed++;
}
process.exit(failed ? 1 : 0);
