/**
 * The AI Copilot's saved conversations (`/api/ai/sessions`): one row per
 * conversation, private to the signed-in user, listing newest first.
 *
 * `AI-24`: reopening, renaming and deleting are the same routes they always
 * were; this contract is where the console now reaches them. The server owns
 * the title — it derives one from the first message until the person renames
 * it, and a renamed title sticks through later saves — so every call that
 * writes returns the title it kept.
 */

/** One conversation as the History list shows it (no messages). */
export type ChatSessionSummary = {
  id: string
  title: string
  /** `ask` or `build`. */
  mode: string
  updatedAt?: string
  /** The conversation made a chart. */
  chartCreated?: boolean
  /** The last message, truncated server-side; still markdown. */
  preview?: string
}

/** One conversation in full. Message shapes are the chat's own (`Msg`). */
export type ChatSession = {
  id: string
  title: string
  mode: string
  messages: unknown[]
  updatedAt?: string
}

/** One page of the History list, with per-mode counts for the same search. */
export type ChatSessionPage = {
  sessions: ChatSessionSummary[]
  hasMore: boolean
  nextOffset: number | null
  counts: Partial<Record<string, number>>
}

export type ListChatSessionsQuery = {
  /** Matched against the title and the conversation text. */
  q?: string
  /** `ask`, `build`, or neither for both. */
  mode?: string
  limit?: number
  offset?: number
}

export type SaveChatSessionInput = {
  /** Omit to start a conversation; a deleted or foreign id is a 404. */
  id?: string
  mode?: string
  messages: unknown[]
}

/** What a write answers: the id it kept and the title now stored. */
export type ChatSessionSaved = {
  ok: boolean
  id: string
  title: string
}

export interface ChatSessionService {
  /** A page of the caller's conversations, newest first. */
  list(query?: ListChatSessionsQuery, signal?: AbortSignal): Promise<ChatSessionPage>
  /** One of the caller's conversations in full. A foreign or deleted id is `not_found`. */
  get(id: string, signal?: AbortSignal): Promise<ChatSession>
  /** Saves the conversation, starting one when `input.id` is absent. */
  save(input: SaveChatSessionInput, signal?: AbortSignal): Promise<ChatSessionSaved>
  /** Renames a conversation; the new title is locked against later saves. */
  rename(id: string, title: string, signal?: AbortSignal): Promise<ChatSessionSaved>
  /** Soft-deletes one of the caller's conversations. */
  remove(id: string, signal?: AbortSignal): Promise<void>
}
