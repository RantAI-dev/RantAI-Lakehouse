# Embedding a chart or a dashboard

| | |
| --- | --- |
| Module | Dashboards (embedding); touches Administration & Security |
| Backlog | `BI-26`, with `AI-11` (assistant parity: embed, with approval) |
| Spec | `docs/core/specs/bi-26.md`, `docs/core/specs/ai-11.md` |
| Status | Ready. The owner signed every "before building" decision on 2026-10-18 (all 19 recommendations accepted); the rest are confirmed at the phase 4 QA |
| Plan | `docs/superpowers/plans/2026-10-18-bi-26-embedding.md` |
| Blocked by | `BI-18` (locking a dashboard filter in an embed: the filters of part A, merged in #104). Also needs PR #201 (`SEC-12` review fixes: embed settings kept outside the dashboard row) merged first; see the plan |

## Problem

A customer's site can show one of our dashboards in a frame, signed by the
customer's server (`SEC-12` made those tokens expire, withdrawable, and
limited to listed sites). What is left:

- A single-chart embed is the dashboard embed with `?chart=` added to the
  address. Remove it and the whole dashboard shows.
- The frame always says "Rantai Lake". The host cannot pick light or dark,
  hide our name, or let its own background show through.
- A filter can only be locked by the token. The viewer cannot be given a
  filter to change, and a filter cannot be kept out of sight.
- The frame has a fixed height, so the host page gets a scroll bar inside
  a scroll bar, or empty space.

## What the user can do when this is done

1. Embed one chart. The token names the chart; nothing else can be shown with it.
2. Choose light, dark, or the dashboard's own look; show or hide our name; and let the host page's background show through.
3. For each dashboard filter, choose: locked (the token must set it and the viewer cannot change it), editable (the viewer sees and changes it), or hidden (not shown; the dashboard's saved value applies).
4. Let the frame grow and shrink with its content: the embed tells the host page its height, and a ten-line snippet in the Share dialog sets it.
5. Copy a ready embed snippet from the Share dialog that shows all of the above.

And ask the assistant to switch embedding on (it asks a person to approve first).

## Not included

- An embedding SDK or React components (owner decision, spec).
- Editing inside an embed (backlog row).
- The customer's own logo in the frame. They can place it on their own page around the frame.
- Downloads from an embed (`BI-1` decision, unchanged).
- Rotating `EMBED_SECRET` with an overlap (`SEC-12`, not included there either).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A token names either a dashboard or one chart. A chart token gets that chart's data and nothing else from the server; the address cannot change that | Spec | Decided by the spec |
| 2 | A dashboard token may still show one of its charts with `?chart=`; it already grants the whole dashboard, so this hides nothing | Planner default | Owner to confirm at QA |
| 3 | The look is set in the embed address, not the token: `theme=light`, `dark` or `dashboard`; `logo=none`; `background=transparent`. It is not secret, so the host can change it without signing a new token | Planner default (Metabase does the same) | Owner to confirm at QA |
| 4 | Our name shows unless the address says `logo=none`. Anyone may hide it | Planner default | Signed by the owner, 2026-10-18 |
| 5 | Each dashboard filter (known by its column, as the dashboard knows it) has an embed mode set by a manager in the Share dialog: locked, editable or hidden. A new filter is hidden until someone chooses | Spec (Metabase); the "hidden first" default is the planner's | Owner to confirm at QA |
| 6 | A locked filter must have a value in the token. A token that leaves one out is refused, so a forgotten value never shows all rows | Planner default (fail closed) | Owner to confirm at QA |
| 7 | Whatever value the token sets is locked, whatever the filter's mode: a locked value can only narrow what is shown, never widen it. So tokens signed today (`params: {"region": "Bali"}`) keep working unchanged | Planner default | Signed by the owner, 2026-10-18 |
| 8 | An editable filter starts at the dashboard's saved value and the viewer can change it, unless the token set it (then decision 7 applies). A hidden filter is not shown and uses the saved value | Planner default | Owner to confirm at QA |
| 9 | Height messages go only to the sites on the dashboard's allowed list (`SEC-12` decision 5). Nothing else is sent to the host page | Planner default | Owner to confirm at QA |
| 10 | An embed always reads data as the fixed viewer role, never more. A token cannot name a role or a person; such a claim is ignored. Expiry, withdrawal and allowed sites stay as `SEC-12` decided | `SEC-12`, signed 2026-10-10 | 2026-10-10 |
| 11 | Who sets embed modes and turns embedding on: someone with manage on the dashboard (`BI-19` part B); until part B ships, `dashboard:write`, as today | Planner default | Owner to confirm at QA |
| 12 | Public-link embeds (`/embed/dashboard/<link>`) get the look and the height messages too. Filter modes apply to signed embeds only | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- The host page must add the ten-line snippet for the height to follow the content. Without it the frame keeps the height the host gave it.
- Hiding our name hides it in the frame only; the page source still shows where the frame comes from.
- A token that does not set every locked filter is refused. Sign every locked filter's value.
- Breaking on upgrade: a single-chart embed made by adding `?chart=` to a dashboard token still works, but the token can still show the whole dashboard. For a chart only, sign a chart token.

## Asking the assistant

`AI-11`, folded in here as on the sharing page.

- "Let example.com embed this dashboard." `enable_embed` switches signed
  embedding on and adds the site to the allowed list. It waits for a
  person to approve it (`DEC-8`: making a dashboard embeddable).
- The assistant never signs a token and never sees `EMBED_SECRET`. It
  returns the embed snippet with a placeholder where the customer's server
  puts the token. (This replaces the `SEC-12` page's "the assistant cannot
  configure embeds", now that `DEC-8` says how.)
- "Make the region filter editable in the embed." `set_embed_filter_modes`
  changes what a viewer can do, so it waits for approval too.
- "Withdraw all embed tokens." `withdraw_embed_tokens` narrows access, so it
  asks for a confirm in the chat (question 5 in `questions-for-owner.md`).
- The person asking must have manage on the dashboard, and so must the
  approver.

The standard request set (`AI-15`) gains `AI-11-AC3`. The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-26-AC1` A chart embed cannot be turned into a dashboard by editing the URL.
- `BI-26-AC2` An embed's height matches its content in the host page.
- `BI-26-AC3` A locked filter cannot be changed by the viewer.
- `BI-26-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4).
- `BI-26-AC5` `theme=dark`, `logo=none` and `background=transparent` each change the frame as named. *(proposed)*
- `BI-26-AC6` An editable filter changes the data when the viewer changes it; a hidden filter is not shown; a token that leaves out a locked filter is refused. *(proposed)*
- `BI-26-AC7` An old token that locks `region` by name still shows only that region. *(proposed)*
- `AI-11-AC3` "Let example.com embed this dashboard." waits for approval, then switches embedding on with example.com listed. *(proposed)*
