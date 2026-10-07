import type { SaveChatTermInput } from "@/services/contracts/chat-terms"

/** A click on one option of the chat's question. */
export type AskAnswer = {
  /** The word the chat asked about. */
  term: string
  /** The option the user picked; it becomes the term's meaning and the next message. */
  option: string
  /** The user's own message that made the chat ask. */
  question: string
}

/** The store refuses a longer question (`MAX_QUESTION_CHARS` in `chat_term.rs`). */
const MAX_QUESTION_CHARS = 500

/**
 * The question as the store accepts it: trimmed, then cut to
 * `MAX_QUESTION_CHARS` characters. The store counts characters and not
 * UTF-16 units, so the cut goes through `Array.from` to keep an emoji whole.
 * PR review fix (SHOULD-FIX): the click sent the whole message, so a long
 * question got a 400 and the answer showed as not remembered.
 */
function questionForStore(question: string): string {
  return Array.from(question.trim()).slice(0, MAX_QUESTION_CHARS).join("")
}

/**
 * Stores the picked option as the user's meaning for the term, then sends
 * it as the next message. The store comes first so the reply to this
 * message is the first one that can read the term. A failed store must not
 * lose the answer: the option is sent anyway and the result says it was
 * not remembered. The send is started, not awaited, so the caller can show
 * that line while the answer is on its way.
 */
export async function answerAsk(
  deps: {
    saveTerm: (input: SaveChatTermInput) => Promise<unknown>
    send: (text: string) => Promise<void>
  },
  { term, option, question }: AskAnswer,
): Promise<boolean> {
  let remembered = true
  try {
    await deps.saveTerm({ term, meaning: option, question: questionForStore(question) })
  } catch {
    remembered = false
  }
  void deps.send(option)
  return remembered
}
