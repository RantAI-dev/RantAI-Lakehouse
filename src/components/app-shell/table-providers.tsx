"use client";

import * as React from "react";
import { NuqsAdapter } from "nuqs/adapters/next/app";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

/**
 * Providers the Advanced Data Table stack needs, mounted once in the root
 * layout (`app/layout.tsx`).
 *
 * `NuqsAdapter` backs the table's URL state — filters, sorting, grouping,
 * and paging all live in the query string so a link reproduces exactly
 * what the sender was looking at. `QueryClientProvider` backs the infinite
 * scroll cache.
 *
 * This is additive, not a replacement: the rest of the app fetches through
 * `useService` (`hooks/use-service.ts`), which owns its own abort/loading/
 * error handling and is untouched here. React Query is currently used by
 * the Data Explorer pilot only.
 *
 * The `QueryClient` is created in a `useState` initializer rather than at
 * module scope on purpose. A module-level client is shared by every render
 * on the server, so one visitor's cached rows could be served to another;
 * this way each browser session gets its own, and it still survives
 * re-renders (unlike a bare `new QueryClient()` in the body).
 */
export function TableProviders({ children }: { children: React.ReactNode }) {
  const [queryClient] = React.useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            // Table data is read-only catalog metadata: refetching it
            // every time the window regains focus costs a full
            // catalog assembly server-side (see `catalog.rs`'s
            // `list_body`) for data that rarely changes mid-session.
            // The toolbar's Refresh button is the deliberate way to
            // get fresh rows.
            refetchOnWindowFocus: false,
            refetchOnReconnect: true,
            retry: 3,
          },
        },
      })
  );

  return (
    <NuqsAdapter>
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    </NuqsAdapter>
  );
}
