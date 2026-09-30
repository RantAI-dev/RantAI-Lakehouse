"use client";

import { BarChart3, Check, ChevronDown, Folder, FolderCog, LayoutGrid, Plus } from "lucide-react";
import Link from "next/link";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu";
import { buildFolderTree, type FolderLike, type FolderNode } from "@/lib/folder-tree";
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
 * Dashboards are grouped by folder (plan §4): root dashboards first, then
 * one nested submenu per folder, at most four levels deep.
 */
export function BoardSwitcher({
  boards,
  folders,
  activeId,
  activeName,
  onSelect,
  onCreate,
  onManageFolders,
}: {
  boards: BoardOption[];
  folders: FolderLike[];
  activeId: string;
  activeName: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
  /** Absent when the viewer cannot edit dashboards. */
  onManageFolders?: () => void;
}) {
  const tree = buildFolderTree(folders, boards);
  const item = (b: BoardOption) => (
    <DropdownMenuItem key={b.id} onClick={() => onSelect(b.id)}>
      <Check className={cn("size-4", b.id === activeId ? "opacity-100" : "opacity-0")} aria-hidden />
      <BarChart3 className="size-3.5 opacity-70" aria-hidden />
      <span className="truncate">{b.name}</span>
    </DropdownMenuItem>
  );
  const folderMenu = (node: FolderNode<BoardOption>) => (
    <DropdownMenuSub key={node.folder.id}>
      <DropdownMenuSubTrigger>
        <Folder className="size-3.5 opacity-70" aria-hidden />
        <span className="truncate">{node.folder.name}</span>
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-60">
        {node.children.map(folderMenu)}
        {node.boards.map(item)}
        {node.children.length === 0 && node.boards.length === 0 ? (
          <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">Empty folder</DropdownMenuLabel>
        ) : null}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );

  return (
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
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuGroup>{tree.rootBoards.map(item)}</DropdownMenuGroup>
        {tree.folders.length ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>{tree.folders.map(folderMenu)}</DropdownMenuGroup>
          </>
        ) : null}
        <DropdownMenuSeparator />
        {/* The menu only lists names; managing them (rename, share, delete)
            lives on `/dashboards/browse`, so the switcher needs a way out
            to it. */}
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
        {onManageFolders ? (
          <DropdownMenuItem onClick={onManageFolders}>
            <FolderCog className="size-4" aria-hidden />
            Manage folders
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
