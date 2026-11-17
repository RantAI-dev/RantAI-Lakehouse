-- SEC-57: a tool-call approval records WHO asked for it, so the API can
-- refuse a self-approval the same way it already refuses one for a
-- catalog access request.
--
-- `0040_access_requests.sql` added `approval_item.requested_by_user_id`
-- for exactly ONE kind: a `kind = 'access'` row always names its human
-- requester, and its shape CHECK (`approval_item_kind_shape_check`) then
-- FORCED `requested_by_user_id IS NULL` on every `kind = 'tool_call'`
-- row -- the reason that migration gives is "a digital employee acted",
-- so no single human requester existed to name.
--
-- That is the gap SEC-57 closes. `routes::agents::decide_approval` never
-- compared the approver with the requester, so a Platform Admin could ask
-- the assistant to delete a chart and approve the deletion themselves;
-- and the requester's id was not even stored, so the API could not refuse
-- it. The requester is now recorded, and the refusal lives in one
-- `pub(crate)` helper both decide routes call
-- (`routes::agents::refuse_own_request`).
--
-- This migration WIDENS the shape constraint; it does not edit `0040`
-- (an applied migration is never edited, never renumbered). A
-- `kind = 'tool_call'` row MAY now name a requester (an `app_user.id` --
-- the same FK `0040` added) and MAY leave it NULL; a `kind = 'access'`
-- row keeps its old shape exactly, because an access request always has
-- a human behind it.
--
-- Old rows keep `requested_by_user_id = NULL`: an approval created
-- before this change names no human requester, so it stays decidable by
-- whoever could decide it before (the requester is UNKNOWN, not
-- "nobody"). No backfill is attempted: `agent_run.actor` holds the
-- requester's DISPLAY NAME, and a display name is not an identity.
--
-- Migration number: `0118` is the next free number at `9c066426`
-- (`0117_outside_table_read_day.sql` is the highest applied one).
ALTER TABLE approval_item DROP CONSTRAINT approval_item_kind_shape_check;

ALTER TABLE approval_item
    ADD CONSTRAINT approval_item_kind_shape_check CHECK (
        (kind = 'tool_call' AND employee_id IS NOT NULL)
        OR (kind = 'access' AND employee_id IS NULL AND requested_by_user_id IS NOT NULL)
    );
