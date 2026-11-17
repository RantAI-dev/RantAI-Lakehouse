"use client";

import { ChevronDown, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuGroupLabel, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { isValueGroup, type CalcField } from "@/services/contracts/calc-fields";
import type { Hierarchy } from "@/services/contracts/hierarchies";

/**
 * The source's calculated fields, value groups and hierarchies in one menu
 * (phase 2 QA R3), in place of a chip for each. A click opens the field's
 * editor; the three "New" actions are at the end.
 */
export function CustomFieldsMenu({
  calculated, hierarchies, onEditField, onEditGroup, onEditHierarchy, onNewField, onNewGroup, onNewHierarchy,
}: {
  readonly calculated: readonly CalcField[];
  readonly hierarchies: readonly Hierarchy[];
  readonly onEditField: (field: CalcField) => void;
  readonly onEditGroup: (field: CalcField) => void;
  readonly onEditHierarchy: (hierarchy: Hierarchy) => void;
  readonly onNewField: () => void;
  readonly onNewGroup: () => void;
  readonly onNewHierarchy: () => void;
}) {
  const count = calculated.length + hierarchies.length;
  const mark = (text: string) => <span className="w-14 shrink-0 text-xs text-muted-foreground">{text}</span>;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button type="button" size="sm" variant="outline" className="w-fit">
          Custom fields ({count}) <ChevronDown className="size-3.5" aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-64">
        {count ? (
          <>
            <DropdownMenuGroup>
              <DropdownMenuGroupLabel>On this source</DropdownMenuGroupLabel>
              {calculated.map((f) => (
                <DropdownMenuItem
                  key={f.id} title={isValueGroup(f) ? "Value group" : f.formula}
                  onClick={() => (isValueGroup(f) ? onEditGroup(f) : onEditField(f))}
                >
                  {mark(isValueGroup(f) ? "group" : "fx")}<span className="truncate">{f.name}</span>
                </DropdownMenuItem>
              ))}
              {hierarchies.map((h) => (
                <DropdownMenuItem key={h.id} title={h.levels.join(" > ")} onClick={() => onEditHierarchy(h)}>
                  {mark("hierarchy")}<span className="truncate">{h.name}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
          </>
        ) : null}
        <DropdownMenuItem onClick={onNewField}><Plus aria-hidden /> New calculated field</DropdownMenuItem>
        <DropdownMenuItem onClick={onNewGroup}><Plus aria-hidden /> New value group</DropdownMenuItem>
        <DropdownMenuItem onClick={onNewHierarchy}><Plus aria-hidden /> New hierarchy</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
