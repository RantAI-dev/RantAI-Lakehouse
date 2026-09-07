"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { parseAsInteger, parseAsString, useQueryStates } from "nuqs";
import * as React from "react";

import { DEFAULT_QUERY_KEYS } from "@/components/data-table/data-table-query-keys";
import { useCallbackRef } from "@/hooks/use-callback-ref";
import { getFiltersStateParser, getSortingStateParser } from "@/lib/parsers";
import type { QueryKeys } from "@/types/data-table";

const STORAGE_PREFIX = "app:table-state:";

/** Bounded so a table that genuinely cannot hold the stored keys settles. */
const RESTORE_ATTEMPTS = 5;
const RESTORE_RETRY_MS = 150;

type ClearListener = () => void;
const clearListeners = new Map<string, Set<ClearListener>>();

export function getTableMemoryStorageKey(persistKey: string) {
  return `${STORAGE_PREFIX}${persistKey}`;
}

/**
 * Sidebar and shortcut links point at a bare path. If this path last held
 * table prefs, attach them so the next navigation lands on the filtered URL
 * in one step — restoring after mount flashes the unfiltered page first.
 */
export function rememberTableHref(href: string): string {
  if (typeof window === "undefined") return href;
  if (!href || href === "#") return href;

  try {
    const url = new URL(href, window.location.origin);
    if (url.origin !== window.location.origin) return href;
    // Only rewrite bare paths. A link that already carries query (shared
    // `?roleId=…`, etc.) must keep the sender's params, not our prefs.
    if (url.search) return href;

    const stored = localStorage.getItem(getTableMemoryStorageKey(url.pathname));
    if (!stored) return href;

    url.search = stored.startsWith("?") ? stored : `?${stored}`;
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return href;
  }
}

/**
 * Drop remembered query prefs for a table. Call from Reset — never from a bare
 * empty URL, which also happens mid-navigation before the page unmounts.
 */
export function clearTableMemory(persistKey: string) {
  const storageKey = getTableMemoryStorageKey(persistKey);
  localStorage.removeItem(storageKey);
  clearListeners.get(storageKey)?.forEach((listener) => listener());
}

function subscribeTableMemoryClear(storageKey: string, listener: ClearListener) {
  let set = clearListeners.get(storageKey);
  if (!set) {
    set = new Set();
    clearListeners.set(storageKey, set);
  }
  set.add(listener);
  return () => {
    set!.delete(listener);
    if (set!.size === 0) clearListeners.delete(storageKey);
  };
}

/**
 * The parts of the URL that describe how someone is looking at a table.
 *
 * `page` is deliberately absent: it is a reading position, not a preference.
 * Returning to page 7 of a list that has since shrunk lands on nothing, and
 * nobody comes back to a screen expecting to resume mid-scroll.
 */
function rememberedKeys(keys: QueryKeys) {
  return [
    keys.view,
    keys.search,
    keys.sort,
    keys.filters,
    keys.joinOperator,
    keys.perPage,
    keys.groupBy,
  ];
}

function read(params: URLSearchParams, keys: string[]) {
  const next = new URLSearchParams();
  // Fixed key order, so an unchanged view always serialises to the same string
  // and the write effect has nothing to react to.
  for (const key of keys) {
    const value = params.get(key);
    if (value !== null) next.set(key, value);
  }
  return next.toString();
}

/**
 * Carries a table's view, search, sort and filters across a visit to another
 * page — the URL alone cannot, because the sidebar links to a bare path.
 *
 * The URL still wins whenever it says anything at all, so a link someone shared
 * opens the way they left it rather than the way the recipient last sat.
 *
 * Empty URL on this table clears memory (user removed filters/search). Empty
 * URL after the pathname has already left is a leave-page race — those still
 * keep {@link clearTableMemory} / the unmount flush instead of wiping early.
 */
export function useTableMemory(
  persistKey?: string,
  queryKeys: QueryKeys = DEFAULT_QUERY_KEYS,
  /** The columns this table actually has, so a stale key is not restored. */
  columnIds?: Set<string>,
  /** Page-specific URL params to remember with the table (e.g. `roleId`). */
  extraKeys: string[] = []
) {
  const pathname = usePathname();
  const searchParams = useSearchParams();

  // Freeze the storage identity on mount. During client navigations the URL
  // (and often `usePathname`) updates before this page unmounts — if we
  // re-derived the key from the live pathname we would write the wrong slot.
  const [scopedKey] = React.useState(() => persistKey ?? pathname);
  const storageKey = getTableMemoryStorageKey(scopedKey);

  const keys = React.useMemo(
    () => [...rememberedKeys(queryKeys), ...extraKeys],
    [queryKeys, extraKeys]
  );

  /**
   * The same parsers the controls themselves hold these keys with.
   *
   * Not a detail: nuqs syncs two hooks on one key by handing the writer's
   * **parsed value** to the reader, with no second parse. Restoring `filters`
   * through `parseAsString` therefore put a raw JSON string where the filter
   * list keeps its array — `length` still answered, so the list rendered and
   * then died on `.map`. Same shape of bug waited in `sort` and `perPage`.
   */
  const parsers = React.useMemo(() => {
    const next = {
      [queryKeys.view]: parseAsString,
      [queryKeys.search]: parseAsString,
      [queryKeys.joinOperator]: parseAsString,
      [queryKeys.perPage]: parseAsInteger,
      [queryKeys.sort]: getSortingStateParser(columnIds),
      [queryKeys.filters]: getFiltersStateParser(columnIds),
      [queryKeys.groupBy]: parseAsString,
      ...Object.fromEntries(extraKeys.map((key) => [key, parseAsString])),
    };

    return next;
  }, [queryKeys, columnIds, extraKeys]);

  // Restored through nuqs rather than the router: the search box, filter list
  // and sort list hold these keys through nuqs, and a plain navigation past it
  // puts values in the URL that those controls never hear about.
  const [, setQuery] = useQueryStates(parsers, {
    shallow: false,
    history: "replace",
    scroll: false,
  });

  // nuqs hands back a fresh setter every render, which would re-run the restore
  // effect and cancel its retry timer before the write ever lands.
  const applyQuery = useCallbackRef(setQuery);

  const serialised = read(new URLSearchParams(searchParams.toString()), keys);

  // Last non-empty snapshot — flushed on unmount so a navigation that clears
  // the query string before this page dies cannot drop remembered filters.
  const lastSavedRef = React.useRef("");

  // idle → nothing decided yet; restoring → waiting for the write to land;
  // live → the URL is the truth and worth remembering.
  const phase = React.useRef<"idle" | "restoring" | "live">("idle");
  const pendingRestoreRef = React.useRef<Parameters<
    typeof setQuery
  >[0] | null>(null);
  const restoreAttemptsRef = React.useRef(0);
  const restoreTimerRef = React.useRef(0);

  React.useEffect(
    () =>
      subscribeTableMemoryClear(storageKey, () => {
        lastSavedRef.current = "";
      }),
    [storageKey]
  );

  React.useEffect(() => {
    if (phase.current === "live") return;

    if (serialised) {
      phase.current = "live";
      // Landed — drop the payload so a later Reset is not undone by a retry.
      pendingRestoreRef.current = null;
      return;
    }

    if (phase.current === "idle") {
      const stored = localStorage.getItem(storageKey);
      // Read back through the whitelist rather than replayed as-is: the stored
      // string is caller-writable, and this also drops keys left by an older
      // version of this list.
      const restored = stored
        ? new URLSearchParams(read(new URLSearchParams(stored), keys))
        : null;

      if (!restored || ![...restored.keys()].length) {
        phase.current = "live";
        return;
      }

      phase.current = "restoring";
      restoreAttemptsRef.current = 0;
      pendingRestoreRef.current = Object.fromEntries(
        keys.map((key) => {
          const raw = restored.get(key);
          // `parse` answers null for anything it does not recognise, which
          // clears the key rather than restoring a value no control can hold.
          return [
            key,
            raw === null ? null : (parsers[key]?.parse(raw) ?? null),
          ];
        })
      );
    }

    const payload = pendingRestoreRef.current;
    if (!payload) return;

    // Arriving by a sidebar link, the first write races the navigation that
    // brought us here and loses: the router commits the bare path after nuqs
    // replaced it, leaving the table unfiltered with its prefs still in
    // storage. Re-issue until the params are actually in the URL.
    let cancelled = false;

    const write = () => {
      if (cancelled) return;
      if (read(new URLSearchParams(window.location.search), keys)) {
        pendingRestoreRef.current = null;
        return;
      }

      restoreAttemptsRef.current += 1;
      void applyQuery(payload);

      if (restoreAttemptsRef.current < RESTORE_ATTEMPTS) {
        restoreTimerRef.current = window.setTimeout(write, RESTORE_RETRY_MS);
      }
    };

    write();

    return () => {
      cancelled = true;
      window.clearTimeout(restoreTimerRef.current);
    };
  }, [serialised, storageKey, keys, parsers, applyQuery]);

  React.useEffect(() => {
    if (phase.current === "restoring") {
      if (serialised) phase.current = "live";
      return;
    }
    if (phase.current !== "live") return;

    // Empty URL while still on this table means the user cleared prefs (chip
    // remove, clearing search, etc.) — forget them. Empty URL after pathname
    // has already left this slot is a leave-page race: keep lastSavedRef so
    // the unmount flush can persist for the next visit. Reset also calls
    // clearTableMemory explicitly.
    if (!serialised) {
      if (pathname === scopedKey) {
        clearTableMemory(scopedKey);
      }
      return;
    }

    lastSavedRef.current = serialised;
    localStorage.setItem(storageKey, serialised);
  }, [serialised, storageKey, pathname, scopedKey]);

  React.useEffect(() => {
    return () => {
      if (lastSavedRef.current) {
        localStorage.setItem(storageKey, lastSavedRef.current);
      }
    };
  }, [storageKey]);
}
