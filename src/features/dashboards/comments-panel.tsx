"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { readableErrorMessage } from "@/lib/notify-message";
import { notifyMentionsRead } from "@/lib/mention-events";
import { useService } from "@/hooks/use-service";
// BI-19 part E: features import `@/services` only.
import {
  dashboardService,
  notificationsService,
  type CommentPerson,
  type DashboardComment,
} from "@/services";

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
export function CommentsPanel({ board, onChanged, className }: {
  readonly board: string;
  /** The side panel (`BI-61` T6) gives the list the room it has, with the composer kept at the bottom. */
  readonly className?: string;
  /** Called after a comment was posted, edited or deleted, so a count elsewhere can follow (`BI-61` facts line). */
  readonly onChanged?: () => void;
}) {
  const list = useService((signal) => dashboardService.listComments(board, signal), [board]);
  const [text, setText] = React.useState("");
  const [picked, setPicked] = React.useState<CommentPerson[]>([]);
  const [people, setPeople] = React.useState<CommentPerson[]>([]);
  // BI-19 owner QA 2026-10-28, QA-4: the `@word` whose lookup has finished, so "nobody matches"
  // is never said while the debounced request is still pending (or after it failed).
  const [lookedUp, setLookedUp] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState<{ id: string; text: string } | null>(null);

  // BI-38 D6: opening this dashboard's comments marks its mentions read once
  // the list has loaded (the component is mounted by the open, and a reload
  // after posting is not a second open). The bell is told to reload; a failed
  // mark is silent because the mentions then stay unread, which is the truth.
  const markedRead = React.useRef(false);
  React.useEffect(() => {
    if (list.status !== "success" || markedRead.current) return;
    markedRead.current = true;
    notificationsService
      .markMentionsRead(board)
      .then(() => notifyMentionsRead())
      .catch(() => undefined);
  }, [board, list.status]);

  const word = pendingMention(text);
  // BI-19 E review E-3: each lookup costs a read of every candidate's access on
  // the server, so wait for a pause in typing and cancel the lookup a newer
  // keystroke makes useless.
  React.useEffect(() => {
    if (word === null) { setPeople([]); setLookedUp(null); return; }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      dashboardService
        .commentPeople(board, word, controller.signal)
        .then((found) => { if (!controller.signal.aborted) { setPeople(found); setLookedUp(word); } })
        .catch(() => { if (!controller.signal.aborted) { setPeople([]); setLookedUp(null); } });
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
      onChanged?.();
    } catch (e) {
      setMessage(sentence(e));
    } finally { setBusy(false); }
  }

  const notifyIds = mentionedIds(text, picked);
  const notifyNames = picked.filter((p) => notifyIds.includes(p.id)).map((p) => p.name);
  const nobodyMatches = word !== null && word !== "" && lookedUp === word && people.length === 0;
  const trimmed = text.trim();
  const tooLong = trimmed.length > MAX_COMMENT_CHARS;

  function post() {
    void act(
      () => dashboardService.postComment({ board, body: trimmed, mentions: notifyIds }),
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
    <div className={cn("flex min-h-0 flex-col gap-3", className)} data-slot="comments-panel">
      {list.status === "loading" ? <p className="text-xs text-muted-foreground">Loading comments…</p> : null}
      {list.status === "error" ? (
        <p role="alert" className="text-xs text-destructive">{sentence(list.error)}</p>
      ) : null}
      {list.status === "success" && comments.length === 0 ? (
        <p className="text-sm text-muted-foreground">No comments yet. Start the conversation below.</p>
      ) : null}
      {truncated ? (
        // BI-19 E review E-6c: the list is the newest 200; say so rather than look complete.
        <p className="text-xs text-muted-foreground">Older comments are not shown.</p>
      ) : null}
      <ul className="-mx-1 min-h-0 flex-1 space-y-1 overflow-y-auto px-1" aria-label="Comments">
        {comments.map((c) => (
          <li key={c.id} className="group/comment flex gap-2.5 rounded-lg py-2">
            <Initials name={c.author?.name ?? "?"} />
            <div className="min-w-0 flex-1">
              <div className="flex min-h-6 items-center justify-between gap-2 text-xs text-muted-foreground">
                <span className="min-w-0 truncate">
                  <span className="font-medium text-foreground">{c.author?.name ?? "Deleted user"}</span> · {when(c.createdAt)}
                  {c.editedAt ? " (edited)" : ""}
                </span>
                <span className="flex shrink-0 gap-0.5">
                  {c.canEdit ? (
                    <Button size="xs" variant="ghost" className="text-muted-foreground" disabled={busy} onClick={() => setEditing({ id: c.id, text: c.body })}>
                      Edit
                    </Button>
                  ) : null}
                  {c.canDelete ? (
                    <Button size="xs" variant="ghost" className="text-muted-foreground hover:text-destructive" disabled={busy} onClick={() => void act(() => dashboardService.deleteComment(c.id))}>
                      Delete
                    </Button>
                  ) : null}
                </span>
              </div>
              {editing?.id === c.id ? (
                <div className="mt-1 space-y-1.5">
                  <Textarea
                    aria-label="Edit comment"
                    value={editing.text}
                    onChange={(e) => setEditing({ id: c.id, text: e.target.value })}
                    maxLength={MAX_COMMENT_CHARS}
                  />
                  <div className="flex justify-end gap-1">
                    <Button size="xs" variant="ghost" onClick={() => setEditing(null)}>Cancel</Button>
                    <Button size="xs" disabled={busy || editing.text.trim() === ""} onClick={saveEdit}>Save</Button>
                  </div>
                </div>
              ) : (
                // Plain text: a React child is escaped, never parsed as HTML or Markdown.
                <p className="mt-0.5 text-sm leading-relaxed whitespace-pre-wrap break-words">{c.body}</p>
              )}
            </div>
          </li>
        ))}
      </ul>
      <div className="shrink-0 space-y-1.5">
        {word !== null && people.length > 0 ? (
          <ul aria-label="People you can mention" className="rounded-lg bg-popover p-1 text-sm shadow-md ring-1 ring-foreground/10">
            {people.map((p) => (
              <li key={p.id}>
                <button
                  type="button"
                  className="w-full rounded-md px-2 py-1.5 text-left hover:bg-muted"
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
        {nobodyMatches ? (
          <p role="status" className="text-xs text-muted-foreground">
            Nobody who can see this dashboard matches “{word}”.
          </p>
        ) : null}
        <div className="rounded-xl bg-muted/70 p-1.5 dark:bg-input/30">
          <Textarea
            aria-label="Write a comment"
            placeholder="Write a comment. Type @ to mention someone."
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="min-h-16 resize-none border-0 bg-transparent shadow-none focus-visible:ring-0 dark:bg-transparent"
          />
          <div className="flex items-center justify-between gap-2 pl-2">
            <span className={tooLong ? "text-xs text-destructive" : "text-xs text-muted-foreground tabular-nums"}>
              {trimmed.length.toLocaleString()}/{MAX_COMMENT_CHARS.toLocaleString()}
            </span>
            <Button size="sm" disabled={busy || trimmed === "" || tooLong} onClick={post}>Post</Button>
          </div>
        </div>
        {notifyNames.length > 0 ? (
          <p className="text-xs text-muted-foreground">Will notify: {notifyNames.join(", ")}</p>
        ) : null}
      </div>
      {message ? <p role="alert" className="text-xs text-destructive">{message}</p> : null}
    </div>
  );
}

/** A person's initials in a circle, beside their comment. */
function Initials({ name }: { readonly name: string }) {
  const initials = name.split(/\s+/).map((w) => w[0]).filter(Boolean).slice(0, 2).join("").toUpperCase() || "?";
  return (
    <span aria-hidden className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-full bg-primary/12 text-[10px] font-semibold text-primary dark:text-sky-200">
      {initials}
    </span>
  );
}
