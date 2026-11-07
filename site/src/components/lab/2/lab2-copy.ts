/**
 * Lab 2's words. A throwaway exploration, English only, so they live here
 * rather than in the message files; each is the home page's own (hero
 * headline, the footer's tagline, the top bar's calls to action), so the
 * lab claims nothing the site does not.
 */
export const LAB2 = {
  /** The home hero's headline, set as poster lines: two on wide screens, three on phones. */
  wide: ["Your data has", "a home now."],
  narrow: ["Your data", "has a", "home now."],
  line: "One lakehouse with dashboards built in. Free and open source, on your own servers.",
  install: "Install it",
  talk: "Talk to us",
  installHref: "/#get-started",
  talkHref: "/contact/",
} as const;
