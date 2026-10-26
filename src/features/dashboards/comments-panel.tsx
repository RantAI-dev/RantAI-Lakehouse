"use client";

import * as React from "react";
import { MessageSquare } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { readableErrorMessage } from "@/lib/notify-message";
import { useService } from "@/hooks/use-service";
// BI-19 part E: features import `@/services` only.
import { dashboardService, type CommentPerson, type DashboardComment } from "@/services";

/** `BI-19` part E, decision D15: a comment is 1 to 2000 characters. */
export const MAX_COMMENT_CHARS = 2000;

/** Wait this long after the last keystroke before asking the server for people. */
export const PICKER_DEBOUNCE_MS = 250;

/** The sentence a failed call carries, as the page shows it. */
function sentence(e: unknown): string {
  return readableErrorMessage(e instanceof Error ? e.message : "The comment was not saved.");
}

/**
 * The `@word` being typed at the end of `text`, if any. The picker opens for
 * it; a space or a newline ends it.
 */
export function pendingMention(text: string): string | null {
  const m = /(?:^|\s)@([^\s@]*)$/.exec(text);
  return m ? m[1] : null;
}

/** Put the chosen person where the `@word` was typed. */
export function insertMention(text: string, person: CommentPerson): string {
  return text.replace(/(^|\s)@[^\s@]*$/, `$1@${person.name} `);
}

/**
 * The people to send as mentioned: those picked whose `@Name` is still in the
 * text, once each. Typing a name without picking sends no mention.
 */
export function mentionedIds(text: string, picked: readonly CommentPerson[]): string[] {
  const ids: string[] = [];
  for (const p of picked) {
    if (text.includes(`@${p.name}`) && !ids.includes(p.id)) ids.push(p.id);
  }
  return ids;
}

function when(at: string): string {
  return new Date(at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * The comments of one dashboard (`BI-19` part E). Text is shown as text: it is
 * rendered as a React child (never as HTML or Markdown), with line breaks kept.
 * The @ picker offers only people who can see the dashboard.
 */
export function CommentsPanel({ board }: { readonly board: string }) {
  const list = useService((signal) => dashboardService.listComments(board, signal), [board]);
  const [text, setText] = React.useState("");
  const [picked, setPicked] = React.useState<CommentPerson[]>([]);
  const [people, setPeople] = React.useState<CommentPerson[]>([]);
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState<{ id: string; text: string } | null>(null);

  const word = pendingMention(text);
  // BI-19 E review E-3: each lookup costs a read of every candidate's access on
  // the server, so wait for a pause in typing and cancel the lookup a newer
  // keystroke makes useless.
  React.useEffect(() => {
    if (word === null) { setPeople([]); return; }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      dashboardService
        .commentPeople(board, word, controller.signal)
        .then((found) => { if (!controller.signal.aborted) setPeople(found); })
        .catch(() => { if (!controller.signal.aborted) setPeople([]); });
    }, PICKER_DEBOUNCE_MS);
    return () => { clearTimeout(timer); controller.abort(); };
  }, [board, word]);

  async function act(work: () => Promise<unknown>, done?: () => void) {
    setBusy(true);
    setMessage(null);
    try {
      await work();
      done?.();
      list.reload();
    } catch (e) {
      setMessage(sentence(e));
    } finally { setBusy(false); }
  }

  const trimmed = text.trim();
  const tooLong = trimmed.length > MAX_COMMENT_CHARS;

  function post() {
    void act(
      () => dashboardService.postComment({ board, body: trimmed, mentions: mentionedIds(text, picked) }),
      () => { setText(""); setPicked([]); },
    );
  }

  function saveEdit() {
    if (!editing) return;
    const body = editing.text.trim();
    void act(() => dashboardService.editComment(editing.id, body), () => setEditing(null));
  }

  const comments: DashboardComment[] = list.status === "success" ? list.data.comments : [];
  const truncated = list.status === "success" && list.data.truncated;

  return (
    <div className="space-y-3">
      {list.status === "loading" ? <p className="text-xs text-muted-foreground">Loading comments…</p> : null}
      {list.status === "error" ? (
        <p role="alert" className="text-xs text-destructive">{sentence(list.error)}</p>
      ) : null}
      {list.status === "success" && comments.length === 0 ? (
        <p className="text-xs text-muted-foreground">No comments yet.</p>
      ) : null}
      {truncated ? (
        // BI-19 E review E-6c: the list is the newest 200; say so rather than look complete.
        <p className="text-xs text-muted-foreground">Older comments are not shown.</p>
      ) : null}
      <ul className="max-h-72 space-y-3 overflow-y-auto" aria-label="Comments">
        {comments.map((c) => (
          <li key={c.id} className="rounded-md border border-border p-2 text-sm">
            <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
              <span>
                {c.author?.name ?? "Deleted user"} · {when(c.createdAt)}
                {c.editedAt ? " (edited)" : ""}
              </span>
              <span className="flex gap-1">
                {c.canEdit ? (
                  <Button size="xs" variant="ghost" disabled={busy} onClick={() => setEditing({ id: c.id, text: c.body })}>
                    Edit
                  </Button>
                ) : null}
                {c.canDelete ? (
                  <Button size="xs" variant="ghost" disabled={busy} onClick={() => void act(() => dashboardService.deleteComment(c.id))}>
                    Delete
                  </Button>
                ) : null}
              </span>
            </div>
            {editing?.id === c.id ? (
              <div className="mt-1 space-y-1">
                <Textarea
                  aria-label="Edit comment"
                  value={editing.text}
                  onChange={(e) => setEditing({ id: c.id, text: e.target.value })}
                  maxLength={MAX_COMMENT_CHARS}
                />
                <div className="flex gap-1">
                  <Button size="xs" disabled={busy || editing.text.trim() === ""} onClick={saveEdit}>Save</Button>
                  <Button size="xs" variant="ghost" onClick={() => setEditing(null)}>Cancel</Button>
                </div>
              </div>
            ) : (
              // Plain text: a React child is escaped, never parsed as HTML or Markdown.
              <p className="mt-1 whitespace-pre-wrap break-words">{c.body}</p>
            )}
          </li>
        ))}
      </ul>
      <div className="space-y-1">
        <Textarea
          aria-label="Write a comment"
          placeholder="Write a comment. Type @ to mention someone who can see this dashboard."
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        {word !== null && people.length > 0 ? (
          <ul aria-label="People you can mention" className="rounded-md border border-border bg-background p-1 text-sm">
            {people.map((p) => (
              <li key={p.id}>
                <button
                  type="button"
                  className="w-full rounded px-2 py-1 text-left hover:bg-muted"
                  onClick={() => {
                    setText(insertMention(text, p));
                    setPicked((cur) => (cur.some((x) => x.id === p.id) ? cur : [...cur, p]));
                  }}
                >
                  {p.name}
                </button>
              </li>
            ))}
          </ul>
        ) : null}
        <div className="flex items-center justify-between">
          <span className={tooLong ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
            {trimmed.length}/{MAX_COMMENT_CHARS}
          </span>
          <Button size="sm" disabled={busy || trimmed === "" || tooLong} onClick={post}>Post</Button>
        </div>
      </div>
      {message ? <p role="alert" className="text-xs text-destructive">{message}</p> : null}
    </div>
  );
}

/** The panel in a dialog, opened from the dashboard's toolbar. */
export function CommentsDialog({
  board, dashName, open, onOpenChange,
}: {
  readonly board: string;
  readonly dashName: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <MessageSquare className="size-4" /> Comments on {dashName}
          </DialogTitle>
        </DialogHeader>
        {open ? <CommentsPanel board={board} /> : null}
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Close</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
