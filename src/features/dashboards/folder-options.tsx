"use client";

import { Lock } from "lucide-react";

import { SelectItem, SelectSeparator } from "@/components/ui/select";
import { PERSONAL_OPTION_LABEL, splitFolderOptions } from "@/lib/dashboard-organise";
import { flattenFolders, buildFolderTree, folderPath, type FolderLike } from "@/lib/folder-tree";

/**
 * The options of a folder picker, the one renderer of New dashboard and Move to
 * folder (BI-18 part C review fix (SHOULD-FIX) R17): the personal space first,
 * with a lock and "Personal space (only you)", a separator, then the entry for
 * no folder and the shared folders. `folders` are the ones the caller may file
 * into; the personal space is the one with an owner.
 */
export function FolderOptions({
  folders, noneValue, noneLabel,
}: {
  readonly folders: FolderLike[];
  readonly noneValue: string;
  readonly noneLabel: string;
}) {
  const { personal, shared } = splitFolderOptions(flattenFolders(buildFolderTree(folders, []).folders), folders);
  return (
    <>
      {personal.map(({ folder }) => (
        <SelectItem key={folder.id} value={folder.id}>
          <Lock className="size-3.5" aria-hidden />
          {folder.ownerId ? PERSONAL_OPTION_LABEL : folderPath(folders, folder.id)}
        </SelectItem>
      ))}
      {personal.length > 0 ? <SelectSeparator /> : null}
      <SelectItem value={noneValue}>{noneLabel}</SelectItem>
      {shared.map(({ folder }) => (
        <SelectItem key={folder.id} value={folder.id}>{folderPath(folders, folder.id)}</SelectItem>
      ))}
    </>
  );
}
