/**
 * Pan and zoom of a drawing inside a box (`BI-71` decision 3, the Catalog
 * Lineage map), kept apart from the component so the arithmetic is tested.
 * The drawing is moved with one CSS transform, `translate(x, y) scale(scale)`
 * from its top-left corner: a point `p` of the drawing lands at
 * `x + scale * p` in the box.
 *
 * This is not `geo-view.ts`: that one is ECharts' own roam state for a map
 * tile (a zoom of 1–20 around a geographic centre); this is a DOM drawing
 * whose smallest zoom is the one that shows all of it.
 *
 * It is view state only: nothing here is saved.
 */

export type View = { scale: number; x: number; y: number }
export type Size = { width: number; height: number }

/** The largest zoom (decision 3). */
export const ZOOM_MAX = 2
/** No fit goes below this, however big the drawing: past it nothing can be read. */
export const ZOOM_FLOOR = 0.2
/** One press of a zoom button or key. */
export const ZOOM_STEP = 1.25

/**
 * The zoom that shows the whole drawing inside the box, less `pad` on each
 * side. Never above 1 (a small drawing is not blown up to fill the box) and
 * never below [`ZOOM_FLOOR`]. It is also the smallest zoom allowed: zooming
 * out further would only add empty space.
 */
export function fitScale(content: Size, box: Size, pad: number): number {
  if (content.width <= 0 || content.height <= 0) return 1
  const s = Math.min((box.width - 2 * pad) / content.width, (box.height - 2 * pad) / content.height, 1)
  return Math.max(ZOOM_FLOOR, s)
}

/** The drawing at its fit, centred in the box. */
export function fitView(content: Size, box: Size, pad: number): View {
  const scale = fitScale(content, box, pad)
  return clampView({ scale, x: 0, y: 0 }, content, box, pad)
}

/**
 * A view kept in bounds: the zoom between the fit and [`ZOOM_MAX`], and the
 * drawing where it can be seen. Along an axis where the drawing fits it is
 * centred; along one where it does not, it may be moved until its edge
 * meets the box's padding, and no further, so it can never be lost off
 * screen.
 */
export function clampView(view: View, content: Size, box: Size, pad: number): View {
  const scale = Math.min(ZOOM_MAX, Math.max(fitScale(content, box, pad), view.scale))
  const axis = (at: number, length: number, room: number) => {
    const drawn = length * scale
    if (drawn <= room - 2 * pad) return (room - drawn) / 2
    return Math.min(pad, Math.max(room - pad - drawn, at))
  }
  return {
    scale,
    x: axis(Number.isFinite(view.x) ? view.x : 0, content.width, box.width),
    y: axis(Number.isFinite(view.y) ? view.y : 0, content.height, box.height),
  }
}

/** The view zoomed by `factor` around `point` (box coordinates), which stays where it is. */
export function zoomAt(view: View, factor: number, point: { x: number; y: number }, content: Size, box: Size, pad: number): View {
  const wanted = Math.min(ZOOM_MAX, Math.max(fitScale(content, box, pad), view.scale * factor))
  const px = (point.x - view.x) / view.scale
  const py = (point.y - view.y) / view.scale
  return clampView({ scale: wanted, x: point.x - wanted * px, y: point.y - wanted * py }, content, box, pad)
}

/**
 * The box's height for a box `width` wide: the drawing's own height, as the
 * map had before it could zoom, shrunk by the zoom that fits its width (so
 * a wide map on a phone leaves no empty band under it), up to `cap`. A
 * taller drawing opens fitted inside the cap instead of running down the
 * page.
 */
export function boxHeight(content: Size, width: number, pad: number, cap: number): number {
  const across = content.width > 0 ? Math.min(1, Math.max(ZOOM_FLOOR, (width - 2 * pad) / content.width)) : 1
  return Math.round(Math.min(cap, content.height * across + 2 * pad))
}
