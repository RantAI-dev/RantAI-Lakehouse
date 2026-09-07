"use client";

import { X } from "lucide-react";
import { usePathname, useSearchParams } from "next/navigation";
import { parseAsString, useQueryStates } from "nuqs";
import { useMemo } from "react";

import { useDataTableQueryKeys } from "@/components/data-table/data-table-query-keys";
import { Button } from "@/components/ui/button";
import { clearTableMemory } from "@/hooks/use-table-memory";

/**
 * Clears the narrowing the user applied — search, sort, filters, and any
 * page-specific keys registered as `memoryKeys` / `resetExtraKeys`. View and
 * page size survive (how they prefer to read the table).
 *
 * Absent until there is something to clear: a permanently visible Reset reads
 * as if the table were filtered when it is not.
 */
export function DataTableResetFilters() {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  // The toolbar names these after its own table, so a page with two tables
  // clears the one the button belongs to rather than both.
  const { keys, paginationMode, resetExtraKeys } = useDataTableQueryKeys();

  // `page` rides along because the row it pointed at is gone once the narrowing
  // is, but it is not itself a reason to offer the button.
  const cleared = useMemo(
    () => [
      keys.search,
      keys.sort,
      keys.filters,
      keys.joinOperator,
      ...resetExtraKeys,
      ...(paginationMode === "infinite" ? [] : [keys.page]),
    ],
    [keys, paginationMode, resetExtraKeys]
  );
  const counted = useMemo(
    () => [keys.search, keys.sort, keys.filters, ...resetExtraKeys],
    [keys, resetExtraKeys]
  );

  const parsers = useMemo(
    () => Object.fromEntries(cleared.map((key) => [key, parseAsString])),
    [cleared]
  );

  // Written through nuqs, not the router: the filter and sort lists hold these
  // same keys through nuqs, and a plain navigation past it leaves them showing
  // state the URL no longer has. `shallow: false` so the page's own query key,
  // which it reads off `useSearchParams`, moves with it.
  const [, clear] = useQueryStates(parsers, {
    shallow: false,
    history: "replace",
  });

  if (!counted.some((key) => searchParams.get(key))) return null;

  return (
    <Button
      variant="ghost"
      size="sm"
      className="text-muted-foreground h-7 px-2 font-normal hover:text-foreground"
      onClick={() => {
        // Empty URL alone is not enough to forget prefs (leave-page races look
        // the same). Reset must clear localStorage explicitly.
        clearTableMemory(pathname);
        void clear(Object.fromEntries(cleared.map((key) => [key, null])));
      }}
    >
      <X className="size-3.5" />
      Reset
    </Button>
  );
}
