# Chat asks back

| | |
| --- | --- |
| Module | Data |
| Backlog | `AI-17` |
| Status | Built, waiting for acceptance |
| Plan | Plan files are not kept in the repository. |

## Problem

A word in a question can fit two tables, two columns or two values in the data map. The chat then picks one reading without saying so, and the answer can be about the wrong thing. It also forgets what the person meant the next time. Separately, the data map writes every table in full, so a question about one table pays for the text of all of them.

## What the user can do when this is done

1. Ask the Copilot a question with a word that fits two or more tables, columns or values, and get one question back with two to four options as buttons.
2. Click an option and get the answer for that reading, with the reply saying which reading it took.
3. Ask with the same word in a new chat and not be asked again: the chat uses the meaning the person clicked.
4. Type an answer in the box instead of clicking, and get an answer for that chat only. Nothing is saved.
5. Read, save and delete their own saved words through `GET`, `PUT` and `DELETE /api/ai/terms`.
6. Ask a question that names one table and get an answer from a data map that writes that table in full and the others on one line each.

## Not included

- A page in the console to see or delete the saved words. The three routes exist.
- Buttons in Query Studio's "Natural language" box. It never asks.
- Saving a typed answer.
- Offering `ask_user` to a Digital Employee's run, because nobody is there to answer.
- Sending the table that is open on a Catalog page as a signal for the data map.
- How tables relate.

## Asking the assistant

Yes. The assistant asks by itself when a word is unclear. The user answers by clicking an option or by typing.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A click saves the meaning, the model never does. The model only asks, through a tool that carries the question and its options | none | Owner, 2026-10-07 |
| 2 | The question's words come from the last two user messages together, with no extra model call | none | Owner, 2026-10-07 |
| 3 | An unclear message gets one of three answers: use the one reading there is and say which, ask once with options, or say the data does not cover it | none | Owner, 2026-10-07 |
| 4 | Two switches, `AI_ASK_BACK` and `AI_RELEVANT_TABLES` | on | Owner, 2026-10-07 |
| 5 | Query Studio's box gets no buttons | none | Owner, 2026-10-07 |
| 6 | The annotation write gets the tenant gate, row-filtered tables lose their samples and ranges, and the drafting instructions treat facts as data | none | Owner, 2026-10-07 |

## Limits to tell a customer

- The chat asks once. If its previous message was a question, it takes the most likely reading and says which.
- Options are two to four, each at most 80 characters.
- A saved word is stored per user. Another user is asked again.
- A user keeps at most 100 saved words. The 101st is refused until one is deleted. The 30 newest go into the prompt.
- A word is 1 to 60 characters and stored in lower case. Its meaning is 1 to 200 characters. The question that led to it is at most 500.
- Buttons work only under the last message of a conversation. In an older message, or in a conversation reopened from history, the options show as plain text.
- If saving the clicked answer fails, the option is still sent and the chat says the answer was not remembered.
- The data map matches a question to tables by an equal word, or by a prefix of at least 4 characters, against table and column name parts, sample values, synonyms and description words. When no table matches, every table is written in full as before. When at least one matches, the other tables are written on one line each, even if the whole map would fit.
- The data map is built to a budget of 14,000 characters. When the tables that match a question already use it, the matching tables after that point and all the one-line tables get no description. They are listed by name in the closing "more tables not described here" line.
- With `AI_RELEVANT_TABLES` on, column statistics are queried for every table when the map is built. The built map is cached for 120 seconds, and not cached at all when a masking policy exists. With the switch off, only the tables that fit the budget are queried, as before.
- A saved word brings the words of its meaning into the match only when the question uses it. The question is the last two user messages together. The word counts as used when every one of its words (ignoring words of fewer than 3 characters and common function words) appears in those messages, in any order, or when the word stands as a phrase in one message with no letter, digit or `_` next to it. A word made only of ignored words never matches.
- A table with a policy row filter has no sample values or ranges in the data map, so a question cannot match on them. When the policies cannot be read, no table has them.
- `AI_ASK_BACK=false` removes the `ask_user` tool, the rules about unclear words and the saved words from the prompt. The `/api/ai/terms` routes still answer.
- `AI_RELEVANT_TABLES=false` makes the data map write every table in full.
- Either switch set to a value other than `true` or `false` stops the API at start. Unset or empty means on.

## Acceptance checklist

Run on a running deployment. Mark each Pass, Fail, or Not run with the reason. A step not performed is never Pass. Nothing has to be set first: the migration runs when the API starts and both switches are on when unset. Rows 1 to 6 use the seed data and a word that fits two of its tables.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | In the Copilot, ask a question with a word that fits two tables of the seed data. | The chat asks one question and shows two to four options as buttons. | |
| 2 | Click one option. | The chat answers with that reading and says which reading it took. | |
| 3 | `GET /api/ai/terms`. | The list holds the word with the meaning that was clicked. | |
| 4 | Open a new chat and ask with the same word. | The chat does not ask again and uses the saved meaning. | |
| 5 | Sign in as another user and ask the same. | The chat asks. The first user's answer is not theirs, and `GET /api/ai/terms` for this user does not list it. | |
| 6 | In a third chat with another unclear word, type the answer instead of clicking. Then `GET /api/ai/terms`. | The chat answers. The list gains no row. | |
| 7 | Call `PUT /api/ai/terms` with a `meaning` of 201 characters. | Response is 400 naming `meaning`. | |
| 8 | Call `GET /api/ai/terms` without signing in. | Response is 401. | |
| 9 | Set `AI_ASK_BACK=false` and restart the API (operator). Ask the question from row 1. | The chat shows no buttons and no `ask_user` step. | |
| 10 | Set `AI_ASK_BACK=maybe` and restart the API (operator). | The API stops with a `ConfigError` naming the accepted values. | |
| 11 | In Query Studio's "Natural language" box, ask the question from row 1. | The box answers and shows no buttons. | |
| 12 | Ask a question that names one table of the seed data. Then set `AI_RELEVANT_TABLES=false` and restart the API (operator), and ask the same question. | Both answers use that table. The console does not show the prompt. By design, with the switch on the prompt holds that table in full and every other table on one line, even when the whole map would fit. With the switch off it holds every table in full. | |

The data map change of `AI_RELEVANT_TABLES` has one row above (row 12). The prompt text is not shown in the console, so the row checks what a reader can see: the answer. The unit tests check the prompt text itself.

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
