# BI-53: what the renderer says about the PDF's layout and its time

Measured for `BI-53` (`LH-BI-1-01`, `LH-BI-6-22`, `LH-BI-6-23`) by the
developer of the slice `dashboards-p3`, from the probe of that plan's T7
(`rust/crates/lakehouse-api/tests/pdf_real_gotenberg.rs`). This file holds
numbers that were measured on the build machine and marks as *not measured*
what needs a running product stack. Nothing here is a verdict about the 45
seconds.

## 1. What was measured

- Machine: ARM64 (`aarch64`), 20 CPUs.
- Renderer: `gotenberg/gotenberg:8.37.0` (the tag `docker-compose.yml` pins),
  which runs its own Chromium. The image was pulled for this architecture on
  2026-11-13.
- Date: 2026-11-13.
- Command, in the foreground, on a container that had just started:

  ```text
  docker run -d --rm --name bi53-gotenberg-scratch -p 127.0.0.1:13353:3000 \
    --add-host=host.docker.internal:host-gateway gotenberg/gotenberg:8.37.0 \
    gotenberg --api-timeout=70s --log-level=warn
  GOTENBERG_TEST_URL=http://127.0.0.1:13353 cargo test -p lakehouse-api \
    --test pdf_real_gotenberg -- --ignored --nocapture --test-threads=1 \
    the_renderer_reports_its_window_its_media_and_how_long_a_conversion_takes
  docker stop bi53-gotenberg-scratch
  ```

- The page the renderer was given is one HTML page with a `width:100%` box; it
  reports `window.innerWidth` (`w`), the box's width in CSS pixels (`box`) and
  `matchMedia("print").matches` (`print`) at load (`start`) and 1.5 s later
  (`later`), then sets `window.__lakehouseRenderReady`. The test's own output,
  copied:

  | Orientation | Elapsed ms | `start`: w | `start`: box | `start`: print | `later`: w | `later`: box | `later`: print |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | A4 landscape | 1966 | 780 | 780 | false | 780 | 780 | true |
  | A4 portrait | 1721 | 780 | 780 | false | 780 | 780 | true |

  Test result: `1 passed; 0 failed; 3 filtered out`. The test asserts only that
  each conversion returned a PDF and that two reports arrived; it asserts
  nothing about these numbers.

## 2. What the numbers say about the layout

One row of the plan's table for each orientation, and no other verdict.

| Orientation | Seen | Meaning |
| --- | --- | --- |
| A4 landscape | `print` is `false` at `start` | A chart created early is created in the screen layout: half-width tiles at their grid width. This is finding F1 as QA saw it |
| A4 landscape | `box` at `later` is 780, smaller than the paper's printable width (1027) | Charts are narrower than the printed card: the card is part empty |
| A4 portrait | `box` at `later` is 780, larger than the paper's printable width (697) | Charts are wider than the printed card and are cut at its right edge: `LH-BI-6-22`, `LH-BI-6-23` |

The last row of that table ("`print` is `true` at `start` and `box` equals the
printable width in both orientations") is **not** what was seen, so the plan's
reading stands and T12 is built.

## 3. What the numbers say about the time

- The first conversion (A4 landscape, the container's first request, so it
  includes the browser's own start): 1966 ms.
- The second (A4 portrait): 1721 ms.
- The first minus the second, 245 ms, is the browser's start (one run each, and
  the two runs differ in orientation as well as in order); the second is the
  renderer's own time for this page. Each includes the page's own 1.5 s of
  waiting, so the conversion of this page itself is under 0.5 s.
- No verdict on the 45 seconds QA measured.

## 4. Not measured

The stages of a real dashboard's PDF, on this machine, each **not measured**:

| Stage | Status | Why, and how to measure it |
| --- | --- | --- |
| Slot wait (`slot_wait_ms`) | not measured | Needs a running product stack and `BI-53`'s T6 lines in the API's log; the build machine is shared and no gate starts a stack (D18) |
| Renderer start and page load | not measured | The same: download one PDF of the dashboard of `LH-BI-1-01` on a deployment that runs this branch and read the gap from "pdf render started" to "render data requested" |
| The dashboard's read (`payload_ms`) | not measured | The same: the gap around "render data read" |
| Draw, wait for ready and print | not measured | The same: the gap from "render data read" to "pdf render finished" |

The probe above measured one empty page, not a dashboard: no chart is created,
no data is read, and the page has no tabs. It says nothing about the 45
seconds.

For whoever reads those stage numbers (S2 of `BI-53`):

| Seen | Points at |
| --- | --- |
| `slot_wait_ms` large | two PDFs were already being made |
| long gap from "pdf render started" to "render data requested" | the renderer's start or the console page's load |
| `payload_ms` large | the dashboard's read as that person |
| about 20 seconds or more from "render data read" to "pdf render finished" | the safety wait: a chart was created and never reported that it finished |
