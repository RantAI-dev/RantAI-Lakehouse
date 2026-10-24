import type { CustomSeriesOption, EChartsOption } from "echarts";
import { DEFAULT_PALETTE } from "@/lib/chart-palettes";
import { wallClockMs } from "@/lib/forecast-periods";
import { escapeHtml } from "@/lib/geo-points";
import { MAX_GANTT_BARS } from "@/lib/type-b";

/**
 * The ECharts option of the Gantt chart (`BI-16` part C1, plan D3). A pure
 * function of the rows a tile got, so a test can pin every mark.
 *
 * The rows are the statement's own (plan D2, `gantt_sql` in `lakehouse-bi`,
 * which lands with T1): `task`, `group`, `start`, `end` and, when the chart has
 * a progress column, `progress`; one row per task and group, from its earliest
 * start to its latest end, with `end >= start`, at most `MAX_GANTT_BARS + 1`
 * of them (the extra row only tells "more than 1,000"). The server also counts
 * the rows whose end is before their start; it hands the count over as
 * `reversedRows` on the result (a shape the plan leaves to the developer of T1,
 * so this reads it as optional and treats an absent count as none).
 *
 * Times are the report zone's wall clock written without a zone ("2026-03-01"
 * or "2026-03-01 08:30:00"); they are read as naive instants (`Date.UTC`) and
 * the axis is drawn in UTC so nothing is shifted by the browser's own zone.
 * "Today" is the wall clock of the report zone read the same way
 * (`wallClockMs`), which is why a task ending today and the line agree.
 */

type Row = Record<string, unknown>;

const DAY = 86_400_000;
const HOUR = 3_600_000;
const MINUTE = 60_000;
/** Tasks the chart shows before it scrolls (a slider appears beyond this). */
const VISIBLE_TASKS = 20;

const theme = (dark: boolean) => ({
  axis: dark ? "#a1a1aa" : "#71717a",
  split: dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.06)",
  tooltipBg: dark ? "#18181b" : "#ffffff",
  tooltipText: dark ? "#e4e4e7" : "#18181b",
  today: dark ? "#f87171" : "#dc2626",
});

const DATE_TIME = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/;

/** A date or date-time as the server writes it, in naive milliseconds; `null` when it is neither. */
export function ganttTime(v: unknown): number | null {
  if (typeof v !== "string") return null;
  const m = DATE_TIME.exec(v.trim());
  if (!m) return null;
  const ms = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(m[4] ?? 0), Number(m[5] ?? 0), Number(m[6] ?? 0));
  return Number.isFinite(ms) ? ms : null;
}

const two = (n: number) => String(n).padStart(2, "0");

/** A naive instant as `2026-03-01`, with `08:30` when it is not midnight. */
export function ganttText(ms: number): string {
  const d = new Date(ms);
  const day = `${d.getUTCFullYear()}-${two(d.getUTCMonth() + 1)}-${two(d.getUTCDate())}`;
  const h = d.getUTCHours();
  const mi = d.getUTCMinutes();
  return h || mi ? `${day} ${two(h)}:${two(mi)}` : day;
}

/** `5 d`, `2 h 30 min`, `45 min`; a start that is also the end is `0`. */
export function ganttDuration(ms: number): string {
  if (ms <= 0) return "0";
  if (ms >= DAY) {
    const days = ms / DAY;
    return `${Number.isInteger(days) ? days : days.toFixed(1)} d`;
  }
  const hours = Math.floor(ms / HOUR);
  const minutes = Math.round((ms - hours * HOUR) / MINUTE);
  if (hours === 0) return `${minutes} min`;
  return minutes ? `${hours} h ${minutes} min` : `${hours} h`;
}

const percent = (v: unknown): number | null => {
  if (v === undefined || v === null || v === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : null;
};

export type GanttInput = {
  rows: readonly Row[];
  dark: boolean;
  /** The chart's palette, one colour per group (`BI-17` part A colours). */
  colors?: readonly string[];
  /** The report time zone, for the line at today. */
  timeZone: string;
  /** The instant "today" is read at (epoch ms); the clock unless a test fixes it. */
  now?: number;
  /** Rows the server left out because their end is before their start (`reversedRows` on the result). */
  reversedRows?: number;
};

export type GanttDrawn =
  | {
      option: EChartsOption;
      /** Bars drawn, and tasks (rows of the axis). */
      bars: number;
      tasks: number;
      /** The groups, in the order of the legend; empty without a group column. */
      groups: string[];
      /** Whether the line at today is inside the drawn range. */
      todayDrawn: boolean;
      /** The sentences the tile prints under the chart. */
      notes: string[];
    }
  | { message: string };

type Bar = { task: string; group: string; start: number; end: number; progress: number | null };

/** The render function of one group's bars: a rectangle from start to end, with the progress as a solid part of it. */
type RenderApi = {
  value: (dim: number) => number;
  coord: (point: [number, number]) => [number, number];
  size: (point: [number, number]) => [number, number];
};
type RenderParams = { coordSys: { x: number; y: number; width: number; height: number } };

/** Exported for the tests: the drawing of one bar, as the shapes ECharts is handed. */
export function ganttBarItem(color: string, params: RenderParams, api: RenderApi): unknown {
  const row = api.value(0);
  const from = api.coord([api.value(1), row]);
  const to = api.coord([api.value(2), row]);
  const height = api.size([0, 1])[1] * 0.6;
  const cs = params.coordSys;
  // A bar is clipped to the plot area; a one-instant task keeps a sliver so it can be seen and clicked.
  const x = Math.max(from[0], cs.x);
  const right = Math.min(Math.max(to[0], from[0] + 2), cs.x + cs.width);
  const width = right - x;
  if (width <= 0) return undefined;
  const y = from[1] - height / 2;
  const progress = Number(api.value(3));
  const shaded = Number.isFinite(progress);
  const children: unknown[] = [
    { type: "rect", shape: { x, y, width, height, r: 2 }, style: { fill: color, opacity: shaded ? 0.35 : 1 } },
  ];
  if (shaded && progress > 0) {
    children.push({ type: "rect", shape: { x, y, width: width * (progress / 100), height, r: 2 }, style: { fill: color } });
  }
  return { type: "group", children };
}

export function ganttOption(i: GanttInput): GanttDrawn {
  const bars: Bar[] = [];
  let reversed = i.reversedRows ?? 0;
  let withoutDates = 0;
  for (const r of i.rows) {
    const start = ganttTime(r.start);
    const end = ganttTime(r.end);
    if (start === null || end === null) {
      withoutDates += 1;
    } else if (end < start) {
      // The statement filters these out (plan D2); a row that got here anyway is counted, not drawn.
      reversed += 1;
    } else {
      bars.push({
        task: String(r.task ?? ""), group: r.group === null || r.group === undefined ? "" : String(r.group),
        start, end, progress: percent(r.progress),
      });
    }
  }
  if (!bars.length) return { message: reversed + withoutDates > 0 ? "No task has a start and an end in that order" : "No data" };

  bars.sort((a, b) => a.start - b.start || a.end - b.end);
  const more = bars.length > MAX_GANTT_BARS;
  const shown = more ? bars.slice(0, MAX_GANTT_BARS) : bars;

  const tasks = [...new Set(shown.map((b) => b.task))];
  const groups = [...new Set(shown.map((b) => b.group))].filter((g) => g !== "");
  const grouped = groups.length > 0;
  const t = theme(i.dark);
  const palette = i.colors?.length ? i.colors : DEFAULT_PALETTE;
  const colorOf = (g: string) => palette[Math.max(0, groups.indexOf(g)) % palette.length] ?? DEFAULT_PALETTE[0];

  const lo = Math.min(...shown.map((b) => b.start));
  const hi = Math.max(...shown.map((b) => b.end));
  const today = wallClockMs(i.now ?? Date.now(), i.timeZone);
  const todayDrawn = today >= lo && today <= hi;

  const series: EChartsOption["series"] = (grouped ? groups : [""]).map((g): CustomSeriesOption => {
    const color = colorOf(g);
    return {
      type: "custom", name: g || "Tasks", color,
      encode: { x: [1, 2], y: 0 },
      renderItem: ((params: RenderParams, api: RenderApi) => ganttBarItem(color, params, api)) as unknown as CustomSeriesOption["renderItem"],
      data: shown.filter((b) => b.group === g || !grouped).map((b) => ({
        // The name is what a click on the bar reports: the task, so the drill filters on the task column.
        name: b.task, value: [tasks.indexOf(b.task), b.start, b.end, b.progress ?? "-"], group: b.group,
      })),
    };
  });
  if (todayDrawn) {
    // The line at today is a mark line on an empty line series: a custom series draws its own shapes only.
    series.push({
      type: "line", data: [], silent: true, tooltip: { show: false },
      markLine: {
        silent: true, symbol: "none", animation: false,
        lineStyle: { color: t.today, type: "dashed", width: 1.5 },
        label: { formatter: "Today", color: t.today, fontSize: 11 },
        data: [{ xAxis: today }],
      },
    });
  }

  const option: EChartsOption = {
    useUTC: true,
    textStyle: { color: t.axis, fontFamily: "inherit" },
    legend: grouped ? { data: groups, top: 0, textStyle: { color: t.axis, fontSize: 11 } } : undefined,
    grid: { left: 8, right: 24, top: grouped ? 32 : 16, bottom: tasks.length > VISIBLE_TASKS ? 36 : 8, containLabel: true },
    tooltip: {
      trigger: "item", backgroundColor: t.tooltipBg, borderWidth: 0, textStyle: { color: t.tooltipText, fontSize: 12 },
      // Task and group names are customer data in an HTML tooltip: escaped.
      formatter: ((p: { data?: { name?: string; value?: [number, number, number, number | string]; group?: string } }) => {
        const d = p.data;
        if (!d?.value) return "";
        const [, start, end, progress] = d.value;
        const lines = [
          `<strong>${escapeHtml(d.name ?? "")}</strong>`,
          ...(d.group ? [escapeHtml(d.group)] : []),
          `${ganttText(start)} to ${ganttText(end)}`,
          `Duration: ${ganttDuration(end - start)}`,
          ...(typeof progress === "number" ? [`Progress: ${Math.round(progress)}%`] : []),
        ];
        return lines.join("<br/>");
      }) as never,
    },
    xAxis: {
      type: "time",
      axisLabel: { color: t.axis, fontSize: 11, hideOverlap: true },
      axisLine: { lineStyle: { color: t.split } }, splitLine: { lineStyle: { color: t.split } },
    },
    yAxis: {
      type: "category", data: tasks, inverse: true,
      axisLabel: { color: t.axis, fontSize: 11, width: 120, overflow: "truncate" },
      axisLine: { show: false }, axisTick: { show: false }, splitLine: { show: false },
    },
    dataZoom: tasks.length > VISIBLE_TASKS
      ? [{ type: "slider", yAxisIndex: 0, startValue: 0, endValue: VISIBLE_TASKS - 1, right: 0, width: 12, brushSelect: false }, { type: "inside", yAxisIndex: 0 }]
      : undefined,
    series,
  };

  const notes: string[] = [];
  if (more) notes.push(`Showing the first ${MAX_GANTT_BARS.toLocaleString("en-US")} tasks by start`);
  if (reversed > 0) notes.push(`${reversed} ${reversed === 1 ? "task ends" : "tasks end"} before ${reversed === 1 ? "it starts" : "they start"} and ${reversed === 1 ? "is" : "are"} left out`);
  if (withoutDates > 0) notes.push(`${withoutDates} ${withoutDates === 1 ? "task has" : "tasks have"} no start or end date and ${withoutDates === 1 ? "is" : "are"} left out`);
  return { option, bars: shown.length, tasks: tasks.length, groups, todayDrawn, notes };
}
