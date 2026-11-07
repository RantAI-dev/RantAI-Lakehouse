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

async function open(w: number, h: number): Promise<Cdp> {
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
  await send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false });
  await send("Page.enable");
  await send("Page.navigate", { url: base });
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

let failed = 0;
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
