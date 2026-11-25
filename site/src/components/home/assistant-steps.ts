/**
 * WEB-5 O19: the steps of the assistant panel's script, shared by the
 * server-rendered panel (`assistant-chat.tsx`, which marks each element
 * with the step it appears at) and the deferred script that plays them
 * (`assistant-script.ts`). The panel's `data-s` lists the steps reached, and
 * `app/globals.css` (`.lk-ast …`) turns each list into a picture.
 */
export const S = {
  type1: 1,
  ask1: 2,
  think1a: 3,
  think1b: 4,
  answer: 5,
  build: 6,
  type2: 7,
  ask2: 8,
  think2: 9,
  card: 10,
  press: 11,
  approved: 12,
} as const;

/** Every step: the finished conversation, which the server renders. */
export const ALL_STEPS = Object.values(S).join(" ");

/**
 * WEB-5 M1 (phone pass): how many steps a phone starts with, the first exchange (question, work,
 * answer and chart: steps 1 to 5). Below 1024 px the panel never sits as an empty box while the
 * first question types; it shows this exchange at once, and the script carries on from Build.
 * Desktop starts from an empty chat, as before.
 */
export const FIRST_EXCHANGE = 5;
