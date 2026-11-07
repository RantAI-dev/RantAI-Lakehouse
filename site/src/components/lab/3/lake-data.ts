/** Lab 3's illustrative questions and answers (no backend; the panel says so). */
type Bar = { label: string; value: number; shown: string };
export type Question = {
  q: string;
  a: string;
  kind: "bars" | "line" | "count";
  data: readonly Bar[];
  total?: string;
};

export const QUESTIONS: readonly Question[] = [
  {
    q: "Which region had the most visitors last quarter?",
    a: "Jakarta led, with 48.2k visitors: 34% of the quarter.",
    kind: "bars",
    data: [
      { label: "Jakarta", value: 48.2, shown: "48.2k" },
      { label: "Surabaya", value: 31.0, shown: "31.0k" },
      { label: "Bandung", value: 27.4, shown: "27.4k" },
      { label: "Medan", value: 19.8, shown: "19.8k" },
      { label: "Bali", value: 16.5, shown: "16.5k" },
    ],
  },
  {
    q: "How did revenue trend this year?",
    a: "Up every month since March; December closed at €1.9M.",
    kind: "line",
    data: [1.1, 1.0, 0.92, 1.05, 1.14, 1.22, 1.3, 1.41, 1.52, 1.6, 1.74, 1.9].map((v, i) => ({
      label: "JFMAMJJASOND"[i],
      value: v,
      shown: `€${v.toFixed(2)}M`,
    })),
  },
  {
    q: "How many orders came in this week?",
    a: "1,284 orders, 12% more than last week.",
    kind: "count",
    total: "1,284",
    data: [140, 162, 155, 188, 201, 214, 224].map((v, i) => ({ label: "MTWTFSS"[i], value: v, shown: String(v) })),
  },
];

