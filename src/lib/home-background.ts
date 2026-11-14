/**
 * Home's background choice (`WEB-17` D2): the lake scene drawn as a dither
 * (the default), as text glyphs, or not at all.
 *
 * The owner asked for a way to turn the scene off so it is not
 * distracting. The choice is kept in this browser (`localStorage`), like the
 * Dashboards table/gallery choice: it is a matter of taste on one screen,
 * not part of the per-account Home layout (`/api/home/layout`), and it must
 * be known before the first paint of Home so the page does not draw the
 * band and then close it up.
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
