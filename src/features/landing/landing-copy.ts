/**
 * Every word and link on the landing page, in one place.
 *
 * Why one file: the plan (`docs/superpowers/plans/2026-10-11-landing-page.md`
 * §2.6 and §4) fixes the copy, and principle 2 forbids inventing claims. A
 * single module makes "is this sentence in the plan?" a diff against one
 * file instead of a hunt through sections. Every capability named here is one
 * `docs/core/PRODUCT.md` §2 marks Have (or Partial, worded as what it does
 * today). There are no customer logos, user counts, uptime or benchmark
 * numbers, testimonials or prices, and none may be added here.
 */

export const LINKS = {
  console: "/login",
  talk: "mailto:contact@rantai.dev?subject=RantAI%20Lakehouse",
  email: "mailto:contact@rantai.dev",
  source: "https://github.com/RantAI-dev/RantAI-Lakehouse",
  rantai: "https://rantai.dev",
  agents: "https://agents.rantai.dev",
  llmops: "https://llmops.rantai.dev",
} as const;

export const NAV_LINKS = [
  { label: "Product", href: "#sources" },
  { label: "How it works", href: "#depths" },
  { label: "Governance", href: "#governance" },
  { label: "Open source", href: "#open" },
] as const;

export const CTA = {
  console: "Open the console",
  talk: "Talk to us",
  source: "Read the source",
} as const;

export const HERO = {
  eyebrow: "RANTAI LAKEHOUSE",
  headline: "Your data has a home now.",
  sub: "One lakehouse with dashboards built in. Bring every source in, keep it in open tables, and ask for answers in plain language.",
  scrollCue: "Scroll to dive",
} as const;

export const SOURCES = {
  headline: "Everything flows in.",
  sub: "Databases, files, streams and APIs land in one place, on a schedule or as they change.",
  /**
   * One pill per item. WEB-1 review round 1 BLOCKER 1: SFTP (not verified end
   * to end, VER-1) and S3-compatible storage (connectors cannot be tested or
   * re-keyed, SRC-6) stay off until those items are done.
   */
  items: [
    "PostgreSQL",
    "MySQL",
    "SQL Server",
    "MongoDB",
    "REST APIs",
    "Kafka",
    "CSV and TSV files",
    "Excel workbooks",
    "Parquet files",
  ],
  caption:
    "Live change capture for PostgreSQL. File upload straight from the console.",
} as const;

export type DepthLayer = {
  id: "bronze" | "silver" | "gold";
  label: string;
  title: string;
  body: string;
  /** Gauge reading, mono. */
  meters: string;
  accent: string;
};

export const DEPTHS = {
  headline: "Three depths.",
  layers: [
    {
      id: "bronze",
      label: "Bronze",
      title: "Raw, kept as it came.",
      body: "Every source lands untouched in open Apache Iceberg tables, so nothing is lost and nothing is locked in.",
      meters: "−10 m",
      accent: "#C8834A",
    },
    {
      id: "silver",
      label: "Silver",
      title: "Cleaned and joined.",
      body: "Pipelines with schedules, dependencies, retries and version history clean the raw data into tables you can trust.",
      meters: "−40 m",
      accent: "#B9C6D3",
    },
    {
      id: "gold",
      label: "Gold",
      title: "Ready for answers.",
      body: "Curated tables built for the questions your business asks, and published as open tables when you switch it on.",
      meters: "−90 m",
      accent: "#F2C14E",
    },
  ] satisfies DepthLayer[],
} as const;

export const ASSISTANT = {
  headline: "Ask, don't dig.",
  sub: "The assistant works with the same tools as the console. It answers questions, builds charts and dashboards, and runs pipelines. Anything risky waits for a person to approve it.",
  caption: "Illustration",
  userOne: "Which region had the most visitors last quarter?",
  answerOne: "Region North had the most visitors last quarter.",
  userTwo: "Pause the nightly sales pipeline.",
  approvalLabel: "Waiting for approval",
  approvalAction: "Pause pipeline",
  approvalTarget: "sales_nightly",
  approve: "Approve",
  decline: "Decline",
  /** Read by assistive tech instead of the decorative mock. */
  srDescription:
    "An illustration of the assistant. A person asks which region had the most visitors last quarter and the assistant answers with a small bar chart. The person then asks to pause the nightly sales pipeline, and the assistant shows a card that waits for a person to approve or decline the change. The region names and numbers are illustrative.",
} as const;

export const DASHBOARDS = {
  headline: "Answers come back up as dashboards.",
  sub: "About two dozen chart types, maps included. Filter, drill down, share by link, embed in your own site, and get an alert when a number crosses a line.",
  caption: "Illustrations, not customer data.",
  tiles: [
    { id: "line", title: "Visits over time", kind: "Line chart" },
    { id: "map", title: "Visits by region", kind: "Map" },
    { id: "sankey", title: "Where visits come from", kind: "Sankey" },
    { id: "kpi", title: "Visits this week", kind: "KPI" },
    { id: "calendar", title: "Busy days", kind: "Calendar heatmap" },
  ],
} as const;

export const GOVERNANCE = {
  headline: "Guarded by default.",
  sub: "Roles decide who sees what. Sensitive columns are classified and masked for everyone who should not see them, and lineage shows which tables each table is built from.",
  viewAs: "View as",
  roles: ["Analyst", "Admin"] as const,
  columns: ["Name", "Email", "City", "Amount"] as const,
  /** Obviously fictional people on the reserved `example.com` domain. */
  rows: [
    { name: "Mira Okafor", email: "mira.okafor@example.com", city: "Lisbon", amount: "1,240.50" },
    { name: "Tomas Reyes", email: "tomas.reyes@example.com", city: "Oslo", amount: "860.00" },
    { name: "Aiko Tanaka", email: "aiko.tanaka@example.com", city: "Nairobi", amount: "2,075.25" },
    { name: "Noor Haddad", email: "noor.haddad@example.com", city: "Lima", amount: "399.90" },
    { name: "Elif Demir", email: "elif.demir@example.com", city: "Hanoi", amount: "1,512.00" },
  ],
  facts: [
    { label: "Proof", text: "Masking proven end to end in an acceptance gate" },
    { label: "Audit", text: "Assistant actions logged; risky ones wait for approval" },
    { label: "Lineage", text: "Lineage from raw table to curated table" },
  ],
} as const;

export const OPEN = {
  headline: "Yours to keep.",
  cards: [
    { n: "01", title: "Open source.", body: "AGPL-3.0. Read every line, run it yourself." },
    {
      n: "02",
      title: "Open tables.",
      body: "Raw data in Apache Iceberg. Any engine that reads Iceberg can read it.",
    },
    {
      n: "03",
      title: "Your servers or ours.",
      body: "Install it on your own infrastructure, or talk to us about running it for you.",
    },
  ],
} as const;

export const FINAL = {
  headline: "Come on in. The data's fine.",
} as const;

export const FOOTER = {
  tagline: "RantAI Lakehouse. One lakehouse with dashboards built in.",
  product: [
    { label: "Everything flows in", href: "#sources" },
    { label: "Three depths", href: "#depths" },
    { label: "Ask, don't dig", href: "#assistant" },
    { label: "Dashboards", href: "#dashboards" },
    { label: "Guarded by default", href: "#governance" },
    { label: "Yours to keep", href: "#open" },
  ],
  rantai: [
    { label: "rantai.dev", href: LINKS.rantai },
    { label: "RantAI Agents", href: LINKS.agents },
    { label: "RantAI LLMOps", href: LINKS.llmops },
  ],
  connect: [
    { label: "GitHub", href: LINKS.source },
    { label: "contact@rantai.dev", href: LINKS.email },
  ],
  legal: "© 2026 RantAI. Depok, West Java, Indonesia.",
} as const;
