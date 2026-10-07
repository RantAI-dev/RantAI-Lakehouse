/**
 * `PUT /api/ai/terms`: the words a user defined for the chat, kept per
 * signed-in user on the server. The chat stores one when the user picks an
 * answer to a question it asked ("what counts as an active customer?"), so
 * the next conversation does not ask again.
 *
 * Only the store is here. The server also lists and deletes terms; the
 * console has no screen for those yet.
 */
export type ChatTerm = {
  term: string
  meaning: string
  /** The user's question that made the chat ask; empty when none was kept. */
  question: string
  updatedAt: string
}

export type SaveChatTermInput = {
  term: string
  meaning: string
  question?: string
}

export interface ChatTermService {
  /** Stores or replaces the caller's meaning for `term`. */
  saveTerm(input: SaveChatTermInput, signal?: AbortSignal): Promise<ChatTerm>
}
