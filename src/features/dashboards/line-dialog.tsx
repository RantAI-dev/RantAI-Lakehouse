"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";

/** A dialog for one line of text: a tab's name, or the text of a heading. */
export function LineDialog({
  title, initial, confirm, label, maxLength, onClose, onSave,
}: {
  title: string;
  initial: string;
  confirm: string;
  /** Placeholder and accessible name of the input. */
  label: string;
  maxLength: number;
  onClose: () => void;
  onSave: (text: string) => void;
}) {
  const [value, setValue] = React.useState(initial);
  const clean = value.trim();
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>{title}</DialogTitle></DialogHeader>
        <Input
          autoFocus
          value={value}
          maxLength={maxLength}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && clean) onSave(clean); }}
          placeholder={label}
          aria-label={label}
        />
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" disabled={!clean} onClick={() => onSave(clean)}>{confirm}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
