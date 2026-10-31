#!/usr/bin/env python3
"""G4 acceptance test for Oracle change capture (`SRC-4` slice 6, plan task T6.3; ADR 0016).

The steps are the shared ones of `cdc_gate.py` (see its module doc). This file holds what only Oracle
has.

# The source

`oracle-g6` (Oracle Database Free, the fixture of the `g6-oracle` gate) switched into `ARCHIVELOG` with
minimal supplemental logging by `oracle-startdb.d/01_archivelog.sh` (Oracle restarts once; `ping` polls
for it). The container database is `FREE`, the pluggable database `FREEPDB1`. The gate, as `SYS`:

- creates the common user `c##dbzuser` with exactly the grants Debezium's documentation lists for
  LogMiner (no `DBA`), in the container database with `CONTAINER=ALL`, on the default `USERS`
  tablespace;
- creates the schema `G4SRC` in the pluggable database with the two identical tables `ORDERS` (current
  row per key) and `ORDERS_HISTORY` (history), 20,000 seeded rows each, both with `(ALL) COLUMNS`
  supplemental logging, and a third table `ORDERS_PLAIN` WITHOUT it (the setup-check case, T6.5).

The connector dials as `c##dbzuser` to `FREE` with `pdbName = FREEPDB1`.

# Encryption (decision S6-c)

`sslMode = require` makes the JDBC driver REQUIRE Oracle native network encryption (AES256, a SHA256
checksum). The gate proves it on the server: as `SYS`, `v$session_connect_info.network_service_banner`
of every session of `C##DBZUSER` names the AES256 encryption adapter and the SHA256 checksum adapter.

# A wrong password

The connection test goes through Kafka Connect's config validation (Amendment 10, S10-a). A second
connector is registered with the right password and its stored password is changed afterwards (Kafka Connect
refuses to create a connector whose login fails); its test must answer the fixed sentence for a refused connection (Debezium does not say it was the login).

# Tables

An Oracle change-capture connector's tables are typed by hand (Amendment 10, S10-c): the discovery
routes answer `not_supported` and the `discover` cell is `refused` in `ops/gates/coverage.json`.
"""

from __future__ import annotations

import os

import cdc_gate
import requests

HOST = "oracle-g6"
PORT = 1521
CDB = "FREE"
PDB = "FREEPDB1"
CDC_USER = "c##dbzuser"
SCHEMA = "G4SRC"
# Debezium's documented LogMiner grants for a common user in a container database
# (https://debezium.io/documentation/reference/stable/connectors/oracle.html#creating-users-for-the-connector).
SYSTEM_GRANTS = (
    "CREATE SESSION", "SET CONTAINER", "FLASHBACK ANY TABLE", "SELECT ANY TABLE", "SELECT_CATALOG_ROLE",
    "EXECUTE_CATALOG_ROLE", "SELECT ANY TRANSACTION", "LOGMINING", "CREATE TABLE", "LOCK ANY TABLE",
    "CREATE SEQUENCE",
)
OBJECT_GRANTS = (
    "SELECT ON V_$DATABASE", "EXECUTE ON DBMS_LOGMNR", "EXECUTE ON DBMS_LOGMNR_D", "SELECT ON V_$LOG",
    "SELECT ON V_$LOG_HISTORY", "SELECT ON V_$LOGMNR_LOGS", "SELECT ON V_$LOGMNR_CONTENTS",
    "SELECT ON V_$LOGMNR_PARAMETERS", "SELECT ON V_$LOGFILE", "SELECT ON V_$ARCHIVED_LOG",
    "SELECT ON V_$ARCHIVE_DEST_STATUS", "SELECT ON V_$TRANSACTION",
)
ENCRYPTION_ADAPTER = "AES256 encryption service adapter"
CHECKSUM_ADAPTER = "SHA256 crypto-checksumming service adapter"
# Debezium 3.7.0.Final answers "Unable to connect (primary): Failed to resolve Oracle database version" for a refused
# login, an unreachable server and a refused encryption alike (the Oracle code is only in the cause), so the API cannot
# say which: the fixed sentence of `cdc_oracle::OracleTestFailure::ConnectionRefused`.
WRONG_LOGIN_SENTENCE = "Oracle connection failed: the connection or the login was refused."
# The fixed line of `cdc_oracle::UNENCRYPTED_SETUP_READ` (Amendment 11, S11-c).
UNENCRYPTED_LINE = (
    "Setup checks are read over unencrypted connections (two sessions: the container database and the "
    "pluggable database); the password is not sent in clear text, the sessions are not encrypted."
)
# The settings of `cdc_setup_check::oracle_checks` for the gate's two selected tables.
EXPECTED_SETTINGS = (
    "ARCHIVELOG", "Supplemental logging", f"All-column logging on {SCHEMA}.ORDERS",
    f"All-column logging on {SCHEMA}.ORDERS_HISTORY", "LogMiner privileges",
)


def _connect(service: str):
    import oracledb

    return oracledb.connect(
        user="sys", password=os.environ["ORACLE_G6_PASSWORD"], dsn=f"{HOST}:{PORT}/{service}",
        mode=oracledb.AUTH_MODE_SYSDBA,
    )


class OracleSource(cdc_gate.Source):
    cdc_type = "Oracle CDC"
    gate = "g4-oracle"
    catalog_db = "icecat_g4_oracle"
    schema = SCHEMA
    host_for_registry = f"{HOST}:{PORT}"
    now = "SYSTIMESTAMP"
    needs_schema_history = True
    # Oracle restarts once to enter ARCHIVELOG, after the image's own start.
    ping_timeout_seconds = 900
    discover_refused = True
    own_setup_checks = True
    bronze_id = "toInt64(id)"

    def __init__(self) -> None:
        # The common user's and the schema owner's password: a throwaway of this gate's stack.
        self.password = os.environ["G4_ORACLE_CDC_PASSWORD"]

    def ping(self) -> None:
        conn = _connect(CDB)
        try:
            cur = conn.cursor()
            cur.execute("SELECT log_mode FROM v$database")
            mode = cur.fetchone()[0]
            cur.execute("SELECT open_mode FROM v$pdbs WHERE name = :1", [PDB])
            row = cur.fetchone()
            if mode != "ARCHIVELOG" or row is None or row[0] != "READ WRITE":
                raise cdc_gate.G4Failure(f"Oracle is not ready: log_mode={mode}, {PDB}={row}")
        finally:
            conn.close()

    def _run(self, statements: list[str], service: str = PDB) -> None:
        conn = _connect(service)
        try:
            cur = conn.cursor()
            for statement in statements:
                cur.execute(statement)
        finally:
            conn.close()

    def execute(self, sql: str) -> None:
        # One connection per statement: the change writer runs in its own thread. A thin-mode
        # connection does not commit by itself.
        conn = _connect(PDB)
        try:
            conn.cursor().execute(sql)
            conn.commit()
        finally:
            conn.close()

    def rows(self, sql: str) -> list[tuple[str, ...]]:
        conn = _connect(PDB)
        try:
            cur = conn.cursor()
            cur.execute(sql)
            return [tuple("" if v is None else str(v) for v in row) for row in cur.fetchall()]
        finally:
            conn.close()

    def qualified(self, table: str) -> str:
        return f"{SCHEMA}.{table}"

    def object_name(self, mode: str) -> str:
        # Oracle's own spelling, which Debezium matches and the API requires (S6-e, S10-c).
        return f"{SCHEMA}.{cdc_gate.TABLES[mode].upper()}"

    def numbers(self, lo: int, hi: int) -> str:
        return f"SELECT g FROM {SCHEMA}.g4_numbers WHERE g BETWEEN {lo} AND {hi}"

    def dial(self) -> dict:
        return {
            "driver": "oracle", "host": HOST, "port": PORT, "database": CDB, "pdbName": PDB,
            "user": CDC_USER, "sslMode": "require",
        }

    def _create_common_user(self) -> None:
        conn = _connect(CDB)
        try:
            cur = conn.cursor()
            try:
                cur.execute(f"DROP USER {CDC_USER} CASCADE")
            except Exception:  # noqa: BLE001 -- it does not exist on a first run
                pass
            cur.execute(
                f'CREATE USER {CDC_USER} IDENTIFIED BY "{self.password}" DEFAULT TABLESPACE users '
                "QUOTA UNLIMITED ON users CONTAINER=ALL"
            )
            for grant in SYSTEM_GRANTS:
                cur.execute(f"GRANT {grant} TO {CDC_USER} CONTAINER=ALL")
            for grant in OBJECT_GRANTS:
                cur.execute(f"GRANT {grant} TO {CDC_USER} CONTAINER=ALL")
        finally:
            conn.close()

    def setup(self) -> None:
        self._create_common_user()
        conn = _connect(PDB)
        try:
            cur = conn.cursor()
            try:
                cur.execute(f"DROP USER {SCHEMA} CASCADE")
            except Exception:  # noqa: BLE001 -- it does not exist on a first run
                pass
            cur.execute(f'CREATE USER {SCHEMA} IDENTIFIED BY "{self.password}" QUOTA UNLIMITED ON users')
            cur.execute(f"CREATE TABLE {SCHEMA}.g4_numbers (g NUMBER(18) PRIMARY KEY)")
            cur.execute(f"INSERT INTO {SCHEMA}.g4_numbers SELECT LEVEL FROM dual CONNECT BY LEVEL <= 110000")
            for table in (*cdc_gate.TABLES.values(), "orders_plain"):
                # NUMBER(18) keeps the key a 64-bit integer in Debezium's events (a wider NUMBER would
                # arrive as text under decimal.handling.mode=string).
                cur.execute(
                    f"CREATE TABLE {SCHEMA}.{table} (id NUMBER(18) PRIMARY KEY, customer VARCHAR2(100), "
                    "amount NUMBER(12,2), status VARCHAR2(50), updated_at TIMESTAMP(6))"
                )
                if table != "orders_plain":
                    cur.execute(
                        f"INSERT INTO {SCHEMA}.{table} SELECT g, 'seed_' || g, MOD(g, 1000) + 0.5, 'seed', SYSTIMESTAMP "
                        f"FROM {SCHEMA}.g4_numbers WHERE g BETWEEN 1 AND {cdc_gate.SEED_ROWS}"
                    )
                    cur.execute(f"ALTER TABLE {SCHEMA}.{table} ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS")
            conn.commit()
        finally:
            conn.close()

    def add_column_and_observe(self) -> None:
        """A new column. Debezium's default `log.mining.strategy` is `online_catalog`; whether it
        captures the column is what this step shows (Amendment 9: if it does not, stop and report)."""
        self._run([f"ALTER TABLE {SCHEMA}.orders ADD (note VARCHAR2(100))"])
        self.execute(f"UPDATE {SCHEMA}.orders SET note = 'schema_note' WHERE id = 2")

    # ── The setup checks (Amendment 11) ─────────────────────────────────────

    def _test_and_wait_for_checks(self, connector_id: str) -> list[dict]:
        """`POST .../test`: Kafka Connect's validation answers at once, passed and `pending` (the
        setup facts are still being read, Amendment 11, S11-a), with the S11-c line in the message;
        then `GET .../discovery/test` until the orchestrator's read ends, and its `setupChecks`."""
        body = cdc_gate.test_connector(connector_id)
        if body.get("supported") is not True or body.get("ok") is not True:
            raise cdc_gate.G4Failure(f"the connection test did not pass, so no setup check was read: {str(body)[:300]}")
        if body.get("pending") is not True:
            raise cdc_gate.G4Failure(f"a passed test should wait for its setup checks (pending): {str(body)[:300]}")
        if UNENCRYPTED_LINE not in str(body.get("message")):
            raise cdc_gate.G4Failure(f"the test result lacks the unencrypted-read line: {body.get('message')!r}")

        def done():
            polled = cdc_gate.API.get(f"{cdc_gate.API_URL}/api/connectors/{connector_id}/discovery/test", timeout=30)
            if not polled.ok:
                raise cdc_gate.G4Failure(f"GET .../discovery/test answered {polled.status_code} {polled.text[:200]}")
            answer = polled.json()
            return answer if answer.get("state") != "running" else None

        answer = cdc_gate._wait_for("the orchestrator's setup read", done, 180, interval_s=3)
        checks = answer.get("setupChecks")
        if answer.get("state") != "done" or not isinstance(checks, list) or not checks:
            raise cdc_gate.G4Failure(f"the setup read did not end with checks: {str(answer)[:300]}")
        return checks

    def _diagnose_setup_reads(self) -> None:
        """When a setup check came back `unchecked`, run the orchestrator's dictionary statements as the
        connector's own login from here and print each outcome (row count, or the Oracle error code), so
        the CI log says which statement the login may not run. Diagnostic only: it asserts nothing."""
        import oracledb

        statements = {
            "all_log_groups": "SELECT owner, table_name FROM all_log_groups WHERE log_group_type = 'ALL COLUMN LOGGING'",
            "all_tables": "SELECT owner, table_name FROM all_tables",
        }
        try:
            conn = oracledb.connect(user=CDC_USER, password=self.password, dsn=f"{HOST}:{PORT}/{PDB}")
        except Exception as exc:  # noqa: BLE001
            print(f"[g4] diagnostic: could not sign in as {CDC_USER}: {str(exc)[:200]}", flush=True)
            return
        try:
            for name, sql in statements.items():
                try:
                    cur = conn.cursor()
                    cur.execute(sql)
                    rows = cur.fetchall()
                    mine = [r for r in rows if "G4SRC" in [str(c) for c in r]]
                    print(f"[g4] diagnostic: {name}: {len(rows)} rows, {len(mine)} for G4SRC, first {rows[:2]}", flush=True)
                except Exception as exc:  # noqa: BLE001
                    print(f"[g4] diagnostic: {name}: {str(exc)[:200]}", flush=True)
        finally:
            conn.close()

    def setup_checks_step(self, connector_id: str) -> None:
        """S11-e: the gate source's exact settings are `ok`; a second connector selecting the table
        without all-column logging and a mistyped name has exactly those two `missing`."""
        good = self._test_and_wait_for_checks(connector_id)
        listed = sorted(c["setting"] for c in good)
        expected = sorted(EXPECTED_SETTINGS)
        if listed != expected:
            raise cdc_gate.G4Failure(f"the setup checks list {listed}, expected {expected}")
        not_ok = [c for c in good if c["status"] != "ok"]
        if not_ok:
            self._diagnose_setup_reads()
            raise cdc_gate.G4Failure(f"the gate source should pass every setup check: {cdc_gate._describe(good)}")
        spec = {
            "adapter": "cdc", "ingestMode": "cdc", "dial": self.dial(),
            "sourceObjects": [
                {"name": f"{SCHEMA}.ORDERS_PLAIN", "target": "g4_orders_plain", "loadMode": "incremental_dedup", "primaryKey": ["id"]},
                {"name": f"{SCHEMA}.NO_SUCH_TABLE", "target": "g4_no_such_table", "loadMode": "incremental_dedup", "primaryKey": ["id"]},
            ],
        }
        other = cdc_gate.register_connector("-missing", spec, self.password)
        try:
            checks = self._test_and_wait_for_checks(other)
            missing = {c["setting"]: c for c in checks if c["status"] == "missing"}
            plain = f"All-column logging on {SCHEMA}.ORDERS_PLAIN"
            typo = f"All-column logging on {SCHEMA}.NO_SUCH_TABLE"
            if set(missing) != {plain, typo}:
                raise cdc_gate.G4Failure(f"expected exactly {plain!r} and {typo!r} to be missing, got {cdc_gate._describe(checks)}")
            command = missing[plain].get("command") or ""
            if command != f'ALTER TABLE "{SCHEMA}"."ORDERS_PLAIN" ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;':
                raise cdc_gate.G4Failure(f"the command for the plain table is {command!r}")
            if missing[typo].get("command") or "No table with this name was found" not in str(missing[typo].get("message")):
                raise cdc_gate.G4Failure(f"the mistyped table should read 'not found' with no command: {missing[typo]}")
            others = [c for c in checks if c["setting"] not in missing and c["status"] != "ok"]
            if others:
                raise cdc_gate.G4Failure(f"the other checks should be ok: {cdc_gate._describe(checks)}")
        finally:
            deleted = cdc_gate.API.delete(f"{cdc_gate.API_URL}/api/connectors/{other}", timeout=120)
            if not deleted.ok:
                print(f"[g4] cleanup: DELETE of the setup-check connector answered {deleted.status_code}", flush=True)
        cdc_gate.say("setup checks", "pass", f"gate source: {cdc_gate._describe(good)}; other connector: {plain!r} and {typo!r} missing alone")

    # ── Wrong password and the encryption banner ────────────────────────────

    def extra_steps(self, connector_id: str) -> None:
        self._wrong_password_is_classified()
        self._connection_is_encrypted()

    def _wrong_password_is_classified(self) -> None:
        """Kafka Connect refuses to CREATE a connector whose login fails, so the connector is registered
        with the right password and the stored one is changed afterwards: the next test dials with the
        wrong one and must answer the fixed sentence for a refused connection (Debezium does not say it was the login)."""
        spec = {
            "adapter": "cdc", "ingestMode": "cdc", "dial": self.dial(),
            "sourceObjects": [
                {"name": f"{SCHEMA}.ORDERS", "target": "g4_orders_wrong", "loadMode": "incremental_dedup", "primaryKey": ["id"]}
            ],
        }
        other = cdc_gate.register_connector("-wrong", spec, self.password)
        try:
            cdc_gate.set_last_credential("not_the_password_1")
            resp = cdc_gate.API.post(f"{cdc_gate.API_URL}/api/connectors/{other}/test", timeout=60)
            if not resp.ok:
                raise cdc_gate.G4Failure(f"POST .../test answered {resp.status_code} {resp.text[:200]}")
            body = resp.json()
            if body.get("ok") is not False or WRONG_LOGIN_SENTENCE not in str(body.get("message")):
                raise cdc_gate.G4Failure(f"a wrong password should give {WRONG_LOGIN_SENTENCE!r}, got {str(body)[:300]}")
            cdc_gate.say("wrong password", "pass", str(body.get("message")))
        finally:
            deleted = cdc_gate.API.delete(f"{cdc_gate.API_URL}/api/connectors/{other}", timeout=120)
            if not deleted.ok:
                print(f"[g4] cleanup: DELETE of the wrong-password connector answered {deleted.status_code}", flush=True)

    def _connection_is_encrypted(self) -> None:
        """Every session of `C##DBZUSER` now must carry the AES256 and SHA256 adapters. These are the
        CONNECTOR's sessions. The orchestrator's two setup sessions (the container database and the PDB,
        unencrypted by Amendment 11) are not counted separately: they are opened and closed inside one
        discovery job, which ended before this step (`_test_and_wait_for_checks` waits for it), so none is
        alive to be listed, and one that was would fail this check, which is the behaviour wanted."""
        conn = _connect(CDB)
        try:
            cur = conn.cursor()
            cur.execute(
                "SELECT s.sid, i.network_service_banner FROM v$session s "
                "JOIN v$session_connect_info i ON i.sid = s.sid AND i.serial# = s.serial# "
                "WHERE s.username = 'C##DBZUSER'"
            )
            banners: dict[int, list[str]] = {}
            for sid, banner in cur.fetchall():
                banners.setdefault(sid, []).append(banner or "")
        finally:
            conn.close()
        if not banners:
            raise cdc_gate.G4Failure("no session of C##DBZUSER: the connector is not connected")
        for sid, lines in banners.items():
            # Oracle words the banner "Oracle Advanced Security: AES256 encryption service adapter for Linux ..."
            # (lower-case "encryption"): compare without case.
            joined = "\n".join(lines).lower()
            if ENCRYPTION_ADAPTER.lower() not in joined or CHECKSUM_ADAPTER.lower() not in joined:
                raise cdc_gate.G4Failure(f"session {sid} of C##DBZUSER is not AES256-encrypted with a SHA256 checksum: {lines}")
        cdc_gate.say("encryption", "pass", f"{len(banners)} session(s) of C##DBZUSER, each with {ENCRYPTION_ADAPTER!r} and {CHECKSUM_ADAPTER!r}")


def main() -> int:
    return cdc_gate.run(OracleSource())


if __name__ == "__main__":
    raise SystemExit(main())
