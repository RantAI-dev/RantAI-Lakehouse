-- GOV-1 (every console change is audited, decision "the audit table refuses
-- change in the database"): `audit_event` is append-only, and from this
-- migration the database enforces it. Before, `0024_audit_event.sql` stated
-- the invariant "by convention" and relied on `lakehouse-store`'s `audit`
-- module having no UPDATE or DELETE helper. That stops a careless handler. It
-- does not stop a new query that edits a row to make an outcome look better,
-- or a cleanup that deletes the rows of one actor.
--
-- WHY A TRIGGER, and not a revoked privilege: the compose file gives the API
-- the same database role that ran these migrations (`POSTGRES_USER`), and no
-- migration creates a second role or grants anything. A `REVOKE` on that role
-- would also take the privilege from the role that owns the table, and a role
-- split is a deployment change this migration does not make. A trigger needs
-- no role change and fails the statement with a sentence that names the table.
--
-- WHY `UPDATE` IS NOT REFUSED OUTRIGHT: `run_id` and `approval_id` are foreign
-- keys declared `ON DELETE SET NULL` (`0024`). When an `agent_run` or an
-- `approval_item` row is deleted, `Postgres` performs the "set null" as an
-- `UPDATE` of every `audit_event` row that points at it, and that update runs
-- the user triggers of the table. The audit row must survive such a delete
-- with its link cleared, so the one change the trigger admits is `run_id`
-- and/or `approval_id` going to NULL (or staying as it is). The check compares
-- the two rows as JSON with those two keys removed, so every other column,
-- and every column a later migration adds, is covered without this function
-- being edited. Pointing a link at a different row, or attaching one to a row
-- that had none, is a change of the record and is refused.
--
-- WHY `TRUNCATE` IS GUARDED TOO: it is one more statement trigger on the same
-- function, and it removes every row without running a row trigger, so a
-- delete-only guard would leave the whole table one command from empty. It
-- also fires when `TRUNCATE ... CASCADE` reaches this table through a foreign
-- key, so truncating `agent_run` that way is refused as well.
--
-- LIMIT: this is a guard against mistakes and against the application's
-- statements, not against the database owner. The owner (and a superuser,
-- which is what the compose file's `POSTGRES_USER` is) can still run
-- `ALTER TABLE audit_event DISABLE TRIGGER ...` or drop the trigger, and
-- nothing here records that. Tamper evidence against the owner needs a copy of
-- the trail outside this database, which this migration does not provide.
--
-- No row and no column changes: the triggers apply from the next statement on.

CREATE FUNCTION audit_event_refuse_change()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF (to_jsonb(NEW) - 'run_id' - 'approval_id')
               IS DISTINCT FROM (to_jsonb(OLD) - 'run_id' - 'approval_id')
           OR (NEW.run_id IS NOT NULL AND NEW.run_id IS DISTINCT FROM OLD.run_id)
           OR (NEW.approval_id IS NOT NULL
               AND NEW.approval_id IS DISTINCT FROM OLD.approval_id)
        THEN
            RAISE EXCEPTION 'audit_event is append-only: an UPDATE may only clear run_id or approval_id';
        END IF;
        RETURN NEW;
    END IF;

    RAISE EXCEPTION 'audit_event is append-only: % is refused', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_event_refuse_update
    BEFORE UPDATE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit_event_refuse_change();

CREATE TRIGGER audit_event_refuse_delete
    BEFORE DELETE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit_event_refuse_change();

CREATE TRIGGER audit_event_refuse_truncate
    BEFORE TRUNCATE ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit_event_refuse_change();
