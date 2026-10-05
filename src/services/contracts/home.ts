/**
 * `GET`/`PUT`/`DELETE /api/home/layout`: which cards and which "create"
 * shortcuts Home shows, in what order, kept per signed-in user on the
 * server so it follows them across browsers.
 *
 * The ids are plain strings on purpose. The console owns the catalogue
 * (`lib/home-layout`) and drops ids it does not recognise when it resolves a
 * saved layout; the server only checks shape and size. A card added or
 * retired later therefore needs no migration here.
 *
 * `supported: false` (no Postgres pool for this deployment) is a valid 200
 * body, distinct from `layout: null`, which means this user never saved one
 * and gets the default.
 */
export type HomeLayout = {
  /** Card ids in display order; an empty list is a deliberate "none". */
  cards: string[]
  /** Shortcut ids in display order (at most three). */
  shortcuts: string[]
  /** The dashboard the preview card shows; `null` follows the last opened. */
  previewBoardId: string | null
}

export type HomeLayoutResponse =
  | { supported: true; layout: HomeLayout | null }
  | { supported: false; reason: string }

export interface HomeService {
  getLayout(signal?: AbortSignal): Promise<HomeLayoutResponse>
  saveLayout(layout: HomeLayout, signal?: AbortSignal): Promise<HomeLayoutResponse>
  /** Back to the default layout: a later `getLayout` reports `layout: null`. */
  resetLayout(signal?: AbortSignal): Promise<HomeLayoutResponse>
}
