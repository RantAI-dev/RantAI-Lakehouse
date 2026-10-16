/**
 * Whether the chart builder's type list is open or folded away, remembered per
 * browser (phase 2 QA R9). A convenience only: storage can be missing, full or
 * blocked (private window, preview), and then the builder works as if nothing
 * were remembered.
 */
export type KindListPref = "expanded" | "collapsed"

export const KIND_LIST_KEY = "lh.chartBuilder.typeList"

export function readKindListPref(): KindListPref | null {
  try {
    const v = window.localStorage.getItem(KIND_LIST_KEY)
    return v === "expanded" || v === "collapsed" ? v : null
  } catch {
    return null
  }
}

export function writeKindListPref(pref: KindListPref): void {
  try {
    window.localStorage.setItem(KIND_LIST_KEY, pref)
  } catch {
    // Not remembered; the choice still holds for this dialog.
  }
}
