import type { ColumnSort, RowData } from "@tanstack/react-table";
import type { DataTableConfig } from "@/config/data-table";
import type { FilterItemSchema } from "@/lib/parsers";

export type TablePaginationMode = "pages" | "infinite";

declare module "@tanstack/react-table" {
  // Generic names must match TanStack's declaration for module augmentation.
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface TableMeta<TData extends RowData> {
    queryKeys?: QueryKeys;
    /** Column order, pinning and visibility back to the page's defaults. */
    resetLayout?: () => void;
    /** Active server-side group column id, if any. */
    groupBy?: string | null;
    setGroupBy?: (columnId: string | null) => void;
    /** How rows are loaded — page buttons or infinite scroll. */
    paginationMode?: TablePaginationMode;
    /** Extra URL keys Reset should clear (page-specific filters). */
    memoryKeys?: string[];
  }

  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface ColumnMeta<TData extends RowData, TValue> {
    label?: string;
    placeholder?: string;
    variant?: FilterVariant;
    options?: Option[];
    range?: [number, number];
    unit?: string;
    icon?: React.ComponentType<React.ComponentProps<"svg">>;
    /** Shrink this column to the width required by its content. */
    fitContent?: boolean;
    /** Allow grouping rows by this column (server-side). */
    enableGrouping?: boolean;
  }
}

/**
 * What each piece of table state is called in the URL. Two tables on one route
 * would otherwise read and write each other's params, so a page with more than
 * one names them apart through `useDataTable`'s `queryKeys`.
 */
export interface QueryKeys {
  page: string;
  perPage: string;
  sort: string;
  filters: string;
  joinOperator: string;
  search: string;
  /** Server-side row grouping column id. */
  groupBy: string;
  /** The table/list toggle. Owned by the page, but remembered with the rest. */
  view: string;
}

export interface Option {
  label: string;
  value: string;
  count?: number;
  icon?: React.ComponentType<React.ComponentProps<"svg">>;
}

export type FilterOperator = DataTableConfig["operators"][number];
export type FilterVariant = DataTableConfig["filterVariants"][number];
export type JoinOperator = DataTableConfig["joinOperators"][number];

export interface ExtendedColumnSort<TData> extends Omit<ColumnSort, "id"> {
  id: Extract<keyof TData, string>;
}

export interface ExtendedColumnFilter<TData> extends FilterItemSchema {
  id: Extract<keyof TData, string>;
}
