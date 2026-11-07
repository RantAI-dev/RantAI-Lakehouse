/**
 * Everything on the home page that is not text: links, anchors, colours,
 * gauge readings and the illustrative data. The words are in
 * `src/messages/<locale>/home.json` and `common.json` (WEB-5 D3, D9), so
 * this module holds no sentence a translator would need to see.
 *
 * Brand and product names stay here, untranslated (D3): RantAI, the depth
 * names Bronze, Silver and Gold, GitHub.
 *
 * The rule of the old `landing-copy.ts` still holds (D7): no customer
 * logos, user counts, uptime or benchmark numbers, testimonials or prices.
 * The numbers below belong to illustrations that the page labels as
 * illustrations.
 */

const REPO = "https://github.com/RantAI-dev/RantAI-Lakehouse";

export const LINKS = {
  /**
   * D8: the site has no console to open, so the main call to action is the
   * repository's install steps. `#quickstart` is the README's "Quickstart"
   * heading, the section that installs the stack (confirmed in README.md).
   */
  install: `${REPO}#quickstart`,
  email: "mailto:contact@rantai.dev",
  source: REPO,
  /**
   * DOC-2: the step-by-step user guide. It lives in the public repository, so
   * the site can link to it without the console serving it.
   */
  guide: `${REPO}/blob/main/docs/core/reference/user-guide.md`,
  rantai: "https://rantai.dev",
} as const;

export const CONTACT_EMAIL = "contact@rantai.dev";

/**
 * WEB-5 D21: the four questions the home page answers before the final
 * call, as indexes into `contact.faq.items`: is it open source (free), can
 * it run on our servers, what can it connect to, how do we install it.
 */
export const HOME_FAQ = [1, 2, 3, 7] as const;

/**
 * WEB-5 D20: the three steps of `#get-started`, word for word from the
 * README's Quickstart (README.md, "## Quickstart"): commands are not
 * translated. Each step's words are `home.getStarted.steps.<i>`, and each
 * step is a claim in `content/claims.ts`.
 */
export const GET_STARTED_COMMANDS = [
  ["git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git", "cd RantAI-Lakehouse"],
  ["sh ops/init-env.sh"],
  ["scripts/compose.sh up --build"],
] as const;

/**
 * WEB-5 D17: RantAI's social profiles, at the same URLs as the RantAI site's
 * `siteConfig.links` (rantai-page `src/lib/config.ts`), so the family links
 * to one set. Brand names, not translated. The short links that site keeps
 * (`/github`, `/linkedin`…) are host redirects; a static export cannot
 * redirect, so they wait for a host (D2) and are not faked here.
 */
export const SOCIAL_LINKS = [
  { id: "linkedin", label: "LinkedIn", href: "https://www.linkedin.com/company/rantai-dev/home/" },
  { id: "github", label: "GitHub", href: "https://github.com/RantAI-dev" },
  { id: "instagram", label: "Instagram", href: "https://www.instagram.com/rantaidev" },
  { id: "youtube", label: "YouTube", href: "https://www.youtube.com/@RantAI-dev" },
] as const;
export type SocialId = (typeof SOCIAL_LINKS)[number]["id"];

/** The repository as shown in text, without the scheme. */
export const SOURCE_LABEL = REPO.replace(/^https:\/\//, "");

/**
 * Section anchors of the top bar, in order; labels are `common.nav.<key>`.
 * WEB-5 O6: `spy` lists the sections during which the item is the current
 * one (the scroll spy in `top-bar.tsx`), so every part of the page lights
 * one item: the dive's chapters belong to "How it works", "Get started" to
 * "Open source".
 */
export const NAV_ANCHORS = [
  { key: "product", href: "#sources", spy: ["sources"] },
  { key: "howItWorks", href: "#depths", spy: ["depths", "assistant", "dashboards"] },
  { key: "governance", href: "#governance", spy: ["governance"] },
  { key: "openSource", href: "#open", spy: ["open", "get-started"] },
] as const;

/** The footer's links to the page's own sections; labels are `common.footer.sections.<key>`. */
export const FOOTER_SECTIONS = [
  { key: "sources", href: "#sources" },
  { key: "depths", href: "#depths" },
  { key: "assistant", href: "#assistant" },
  { key: "dashboards", href: "#dashboards" },
  { key: "governance", href: "#governance" },
  { key: "open", href: "#open" },
] as const;

/**
 * WEB-5 D17: the RantAI family, as rantai-page's footer lists it, at the
 * same URLs. Brand names, not translated. Each product shows its official
 * mark (rantai-page `public/logo/*Dark.svg`, copied to `public/logo/`),
 * sized by height at its own ratio; Lakehouse is this site, drawn inline
 * (`MascotMark`) and linked to its own home page.
 */
export const RANTAI_PRODUCTS = [
  { id: "agents", label: "RantAI Agents", href: "https://agents.rantai.dev/", logo: "/logo/agents-dark.svg", ratio: 130 / 100 },
  { id: "claw", label: "RantAI Claw", href: "https://claw.rantai.dev/", logo: "/logo/claw-dark.svg", ratio: 106 / 100 },
  { id: "llmops", label: "RantAI LLMOps", href: "https://llmops.rantai.dev/", logo: "/logo/llmops-dark.svg", ratio: 1 },
  { id: "lakehouse", label: "RantAI Lakehouse", href: null, logo: null, ratio: 196 / 167 },
] as const;

export type DepthId = "bronze" | "silver" | "gold";

export type DepthLayer = {
  id: DepthId;
  /** Product name, not translated. */
  label: string;
  /** Gauge reading. */
  meters: string;
  accent: string;
};

/** The three medallion depths; titles and bodies are `home.depths.layers.<id>`. */
export const DEPTH_LAYERS = [
  { id: "bronze", label: "Bronze", meters: "−10 m", accent: "#C8834A" },
  { id: "silver", label: "Silver", meters: "−40 m", accent: "#B9C6D3" },
  { id: "gold", label: "Gold", meters: "−90 m", accent: "#F2C14E" },
] as const satisfies readonly DepthLayer[];

/** The assistant mock's bar chart; region names are `home.assistant.regions.<key>`. */
export const REGIONS = [
  { key: "north", value: 12400 },
  { key: "south", value: 9100 },
  { key: "east", value: 7800 },
  { key: "west", value: 5200 },
] as const;

export const APPROVAL_TARGET = "sales_nightly";

/** The five dashboard tiles, in rail order; text is `home.dashboards.tiles.<id>`. */
export const TILE_IDS = ["line", "map", "sankey", "kpi", "calendar"] as const;
export type TileId = (typeof TILE_IDS)[number];

/** Obviously fictional people on the reserved `example.com` domain. */
export const GOVERNANCE_ROWS = [
  { name: "Mira Okafor", email: "mira.okafor@example.com", city: "Lisbon", amount: "1,240.50" },
  { name: "Tomas Reyes", email: "tomas.reyes@example.com", city: "Oslo", amount: "860.00" },
  { name: "Aiko Tanaka", email: "aiko.tanaka@example.com", city: "Nairobi", amount: "2,075.25" },
  { name: "Noor Haddad", email: "noor.haddad@example.com", city: "Lima", amount: "399.90" },
  { name: "Elif Demir", email: "elif.demir@example.com", city: "Hanoi", amount: "1,512.00" },
] as const;

/** Column order of the governance table; `masked` columns hide for analysts. */
export const GOVERNANCE_COLUMNS = [
  { key: "name", masked: true },
  { key: "email", masked: true },
  { key: "city", masked: false },
  { key: "amount", masked: false },
] as const;

/**
 * WEB-5 D14, the journey: the depth (in metres, 0 at the surface) at the
 * start of each chapter, by section id, in page order. The page dives from
 * the surface to the deepest point at the assistant, then comes back up to
 * the surface at the final call. The depth gauge reads these, and the
 * page-wide water colour follows them (`lib/depth-journey.ts`). The depths
 * section's own markers stay the three slabs' readings (`DEPTH_LAYERS`).
 */
export const JOURNEY = [
  { id: "hero", depth: 0 },
  { id: "sources", depth: -5 },
  { id: "depths", depth: -10 },
  { id: "assistant", depth: -100 },
  { id: "dashboards", depth: -100 },
  { id: "governance", depth: -50 },
  { id: "open", depth: -15 },
  { id: "final", depth: -3 },
] as const;

/** Where the depth reaches 0 again: this far into the final section, as a fraction of its height. */
export const SURFACE_AT_FINAL = 0.5;

