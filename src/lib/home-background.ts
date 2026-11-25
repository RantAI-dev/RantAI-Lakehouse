/**
 * Home's background choice (`WEB-17` D2): the lake scene drawn as a dither
 * (the default), as text glyphs, or not at all.
 *
 * The owner asked for a way to turn the scene off so it is not
 * distracting. The choice is kept in this browser (`localStorage`), like the
 * Dashboards table/gallery choice: it is a matter of taste on one screen,
 * not part of the per-account Home layout (`/api/home/layout`), and it must
 * be known before the first paint of Home so an Off page never shows the
 * band's colour. Off hides the art only; the layout stays the same (owner
 * QA 2026-11-14).
 *
 * Every storage access is wrapped: in a private window, with storage
 * blocked or over quota, `localStorage` throws, and a background preference
 * is not worth breaking Home for. Reading then falls back to the default and
 * writing keeps the choice for this page view only.
 */
export const HOME_BACKGROUNDS = ["dither", "ascii", "off"] as const
export type HomeBackground = (typeof HOME_BACKGROUNDS)[number]

export const DEFAULT_HOME_BACKGROUND: HomeBackground = "dither"

/** The `localStorage` key. */
export const HOME_BACKGROUND_KEY = "home-background"

export const HOME_BACKGROUND_LABEL: Record<HomeBackground, string> = {
  dither: "Dither",
  ascii: "ASCII",
  off: "Off",
}

/** A stored value as a choice; anything unknown (or nothing) is the default. */
export function parseHomeBackground(raw: string | null | undefined): HomeBackground {
  return HOME_BACKGROUNDS.find((b) => b === raw) ?? DEFAULT_HOME_BACKGROUND
}

type Readable = Pick<Storage, "getItem">
type Writable = Pick<Storage, "setItem">

/** The saved choice, or the default when there is none or storage cannot be read. */
export function readHomeBackground(storage: Readable | undefined): HomeBackground {
  try {
    return parseHomeBackground(storage?.getItem(HOME_BACKGROUND_KEY))
  } catch {
    return DEFAULT_HOME_BACKGROUND
  }
}

/** Remember the choice; `false` when storage refused it (it still applies to this view). */
export function writeHomeBackground(storage: Writable | undefined, value: HomeBackground): boolean {
  try {
    if (!storage) return false
    storage.setItem(HOME_BACKGROUND_KEY, value)
    return true
  } catch {
    return false
  }
}

/** Whether a choice draws the scene at all. */
export function drawsScene(value: HomeBackground): value is Exclude<HomeBackground, "off"> {
  return value !== "off"
}

/** The mark on `<html>` that says Off before React runs: `data-home-bg="off"`. */
export const HOME_BACKGROUND_ATTR = "data-home-bg"

/**
 * Run by the root layout before the first paint (with the request's CSP
 * nonce, like `next-themes`' own script): marks `<html>` when this browser
 * chose Off, so the server-rendered loading look of Home does not show the
 * band's colour before JavaScript runs. A constant: it reads
 * one known key and sets one known attribute, nothing from the request.
 */
export const HOME_BACKGROUND_SCRIPT = `try{if(localStorage.getItem(${JSON.stringify(HOME_BACKGROUND_KEY)})==="off")document.documentElement.setAttribute(${JSON.stringify(HOME_BACKGROUND_ATTR)},"off")}catch(e){}`
