/**
 * A page's open tab or section, kept in one query parameter (`?tab=`,
 * `?section=`) so a refresh or a shared link lands on it (`WEB-3` S3-D5 to
 * S3-D7). The first tab is the bare address: it is never written to the URL.
 * An unknown value falls back to the first tab rather than showing nothing.
 */

/** The tab `raw` names, or `values[0]` when it names none of them. */
export function parseUrlTab<T extends string>(raw: string | null | undefined, values: readonly T[]): T {
  const found = values.find((v) => v === raw)
  return found ?? values[0]
}

/**
 * The address for opening `next`: the other query parameters are kept, and
 * the first tab removes the parameter instead of writing it.
 */
export function urlWithTab<T extends string>(
  pathname: string,
  search: string,
  param: string,
  next: T,
  values: readonly T[]
): string {
  const params = new URLSearchParams(search)
  if (next === values[0]) params.delete(param)
  else params.set(param, next)
  const query = params.toString()
  return query ? `${pathname}?${query}` : pathname
}
