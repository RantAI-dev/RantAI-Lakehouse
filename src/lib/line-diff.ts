/**
 * A line diff between the editor's text and a proposed fix, for Query
 * Studio's "Fix with AI" card. It is pure and computed here, so the server
 * never sees the editor's text in a second form.
 *
 * Lines are compared exactly, trailing spaces included: a line that differs
 * only in trailing spaces is a change the person should see, not a no-op.
 */

export type DiffLine = { kind: "same" | "removed" | "added"; text: string }

/**
 * The lines of `before` and `after`, in order, with each line marked as
 * kept, removed from `before`, or added in `after`. The order follows the
 * longest common subsequence of lines, so a changed line shows as a removed
 * line immediately followed by the added line that replaces it.
 */
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before === "" ? [] : before.split("\n")
  const b = after === "" ? [] : after.split("\n")

  // lcs[i][j] is the length of the longest common subsequence of a[i..] and b[j..].
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () =>
    new Array<number>(b.length + 1).fill(0)
  )
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i]![j] =
        a[i] === b[j]
          ? lcs[i + 1]![j + 1]! + 1
          : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!)
    }
  }

  const out: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i]! })
      i++
      j++
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) {
      out.push({ kind: "removed", text: a[i]! })
      i++
    } else {
      out.push({ kind: "added", text: b[j]! })
      j++
    }
  }
  while (i < a.length) out.push({ kind: "removed", text: a[i++]! })
  while (j < b.length) out.push({ kind: "added", text: b[j++]! })
  return out
}
