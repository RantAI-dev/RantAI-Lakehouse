/**
 * In-session mutable store for mock adapters.
 * Seed data is copied once; creates/updates persist until the page reloads.
 */

export type MutableStore<T extends { id: string }> = {
  list: () => T[]
  get: (id: string) => T | undefined
  prepend: (item: T) => T
  update: (id: string, patch: Partial<T>) => T | undefined
}

export function createStore<T extends { id: string }>(seed: T[]): MutableStore<T> {
  let items = seed.map((item) => ({ ...item }))

  return {
    list() {
      return items.map((item) => ({ ...item }))
    },
    get(id) {
      const found = items.find((item) => item.id === id)
      return found ? { ...found } : undefined
    },
    prepend(item) {
      items = [{ ...item }, ...items]
      return { ...item }
    },
    update(id, patch) {
      const index = items.findIndex((item) => item.id === id)
      if (index < 0) return undefined
      const next = { ...items[index], ...patch, id }
      items = [...items.slice(0, index), next, ...items.slice(index + 1)]
      return { ...next }
    },
  }
}
