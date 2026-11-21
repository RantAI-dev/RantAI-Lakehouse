"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { parseAsInteger, parseAsString, useQueryStates } from "nuqs";
import { Search, X } from "lucide-react";

import { useDebouncedCallback } from "@/hooks/use-debounced-callback";
import { useDataTableQueryKeys } from "@/components/data-table/data-table-query-keys";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";

/**
 * The table's free-text search as a value and a setter: the backend's
 * `search` param, kept in the URL like the rest of the table state, held
 * locally only so typing stays responsive (the URL write is debounced).
 *
 * Split from `DataTableSearch` (`UX-8`) so a page can draw the same search
 * in its own control, the icon-only search of the page frame (`DESIGN.md`
 * §3), without a second copy of this state. Must sit under the toolbar's
 * `DataTableQueryKeysProvider`, which names the table's params.
 */
export function useDataTableSearchValue(shallow = false): {
  value: string;
  update: (next: string) => void;
} {
  // Named by the toolbar's table, so two tables on one route do not type into
  // each other's search box.
  const { keys, paginationMode } = useDataTableQueryKeys();
  const isInfinite = paginationMode === "infinite";
  const parsers = useMemo(
    () => ({
      [keys.search]: parseAsString.withOptions({ shallow }).withDefault(""),
      ...(isInfinite
        ? {}
        : { [keys.page]: parseAsInteger.withOptions({ shallow }).withDefault(1) }),
    }),
    [isInfinite, keys.page, keys.search, shallow]
  );

  const [query, setQuery] = useQueryStates(parsers);
  const search = query[keys.search] as string;
  const [value, setValue] = useState(search);
  const pushed = useRef(search);

  // Adopt a `search` this input did not write — the toolbar's Reset, or the
  // state restored on arrival. Comparing against what we last pushed is what
  // keeps the debounce from being undone mid-word by its own echo.
  // The `pushed.current` guard is what makes this safe: the effect only
  // sets state when `search` changed from somewhere other than this input,
  // so it cannot cascade off its own write. Syncing an uncontrolled input
  // to external URL state is exactly the escape hatch this rule allows.
  useEffect(() => {
    if (search === pushed.current) return;
    pushed.current = search;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setValue(search);
  }, [search]);

  // A different search means the current page number is meaningless.
  const push = useDebouncedCallback(
    (next: string) =>
      setQuery(
        isInfinite
          ? { [keys.search]: next || null }
          : { [keys.search]: next || null, [keys.page]: null }
      ),
    300
  );

  const update = (next: string) => {
    setValue(next);
    pushed.current = next;
    push(next);
  };

  return { value, update };
}

/**
 * Free-text search over whatever the endpoint declared `searchable` — the
 * backend's `search` param, which is ANDed with whatever the filter list does.
 */
export function DataTableSearch({
  placeholder = "Search...",
  label = "Search",
  /** Match canonical table fetch (`useSearchParams`) — prefer false. */
  shallow = false,
}: {
  placeholder?: string;
  label?: string;
  shallow?: boolean;
}) {
  const { value, update } = useDataTableSearchValue(shallow);

  return (
    <InputGroup className="w-56">
      <InputGroupAddon className="pl-2.5">
        <Search className="size-4" />
      </InputGroupAddon>
      <InputGroupInput
        type="search"
        // WebKit hangs its own clear button off `type="search"`, which lands
        // right beside ours. The type stays for what it is worth elsewhere —
        // the announced role, and the search key on a touch keyboard.
        className="[&::-webkit-search-cancel-button]:[-webkit-appearance:none]"
        value={value}
        onChange={(event) => update(event.target.value)}
        placeholder={placeholder}
        aria-label={label}
      />
      {value && (
        <InputGroupAddon align="inline-end" className="pr-1.5">
          <InputGroupButton
            size="icon-xs"
            variant="ghost"
            onClick={() => update("")}
          >
            <X />
            <span className="sr-only">Clear search</span>
          </InputGroupButton>
        </InputGroupAddon>
      )}
    </InputGroup>
  );
}
