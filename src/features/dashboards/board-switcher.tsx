"use client";

import { BarChart3, Check, ChevronDown, Folder, LayoutGrid, Plus } from "lucide-react";
import Link from "next/link";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu";
import { folderPath, groupBoardsByFolder, type FolderLike } from "@/lib/folder-tree";
import { cn } from "@/lib/utils";

export type BoardOption = { id: string; name: string; folderId?: string | null };

/**
 * Picks which dashboard is open, rendered as the page title.
 *
 * This list used to live in the sidebar, but only appeared once you were
 * already on `/dashboards` — so it could never be used to *get* here. That
 * made it page content wearing navigation's clothes. A dashboard is a
 * document rather than a section of the product, so choosing one belongs
 * with the document.
 *
 * Creating a dashboard moved along with it. It was previously only
 * reachable from the sidebar, which put it out of reach for anyone
 * browsing with the sidebar collapsed to icons.
 *
 * The list is flat: every dashboard is one click away, with its folder
 * path beside the name as a hint. It used to nest a submenu per folder,
 * so a dashboard in "Latihan / Material" took three hovers to reach; this
 * menu is for switching quickly, and browsing or managing folders is done
 * on `/dashboards/browse` (QA feedback). The order follows that list's
 * sections: no-folder first, then folder by folder.
 */
export function BoardSwitcher({
  boards,
  folders,
  activeId,
  activeName,
  onSelect,
  onCreate,
}: {
  boards: BoardOption[];
  folders: FolderLike[];
  activeId: string;
  activeName: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
}) {
  const ordered = groupBoardsByFolder(folders, boards, false).flatMap((section) => section.boards);
  const item = (b: BoardOption) => {
    const path = b.folderId ? folderPath(folders, b.folderId) : "";
    return (
      // The path is cut to fit the row; the tooltip carries it in full.
      <DropdownMenuItem key={b.id} title={path ? `${b.name} · ${path}` : b.name} onClick={() => onSelect(b.id)}>
        <Check className={cn("size-4 shrink-0", b.id === activeId ? "opacity-100" : "opacity-0")} aria-hidden />
        <BarChart3 className="size-3.5 shrink-0 opacity-70" aria-hidden />
        <span className="min-w-0 flex-1 truncate">{b.name}</span>
        {path ? <span className="max-w-28 shrink-0 truncate text-xs text-muted-foreground">{path}</span> : null}
      </DropdownMenuItem>
    );
  };

  const activeFolder = boards.find((b) => b.id === activeId)?.folderId;
  const activePath = activeFolder ? folderPath(folders, activeFolder) : "";

  return (
    <span className="flex min-w-0 flex-col">
      {/* Where this dashboard is filed, above its name, so the folder is
          visible without opening the menu. It leads to the list, where
          folders are browsed and managed. */}
      {activePath ? (
        <Link
          href="/dashboards/browse"
          title="Browse all dashboards"
          className="flex w-fit items-center gap-1.5 text-xs font-normal tracking-normal text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        >
          <Folder className="size-3.5" aria-hidden />
          {activePath}
        </Link>
      ) : null}
    <DropdownMenu>
      {/* `asChild`, not `render`: this menu comes from the design system,
          which wraps radix — the local `@/components/ui` primitives wrap
          base-ui and use `render` instead. */}
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Switch dashboard"
          className="group -ml-2 flex min-w-0 items-center gap-1.5 rounded-md px-2 py-0.5 text-left hover:bg-muted/60"
        >
          <span className="truncate text-2xl font-semibold leading-8 tracking-[-0.02em] text-foreground">
            {activeName}
          </span>
          <ChevronDown
            className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
            aria-hidden
          />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        <DropdownMenuGroup className="max-h-72 overflow-y-auto">{ordered.map(item)}</DropdownMenuGroup>
        <DropdownMenuSeparator />
        {/* The menu only lists names; managing them (rename, folders,
            delete) lives on `/dashboards/browse`, so the switcher needs a
            way out to it. */}
        <DropdownMenuItem asChild>
          <Link href="/dashboards/browse">
            <LayoutGrid className="size-4" aria-hidden />
            Browse all dashboards
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onCreate}>
          <Plus className="size-4" aria-hidden />
          New dashboard
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    </span>
  );
}
