#!/bin/bash
# SRC-4 slice 6 (S6-h), GATE-ONLY. gvenzl/oracle-free runs the scripts of
# /container-entrypoint-startdb.d after the database is started, on EVERY start, so this one is
# idempotent: it reads LOG_MODE first and restarts the database into ARCHIVELOG only when it is not
# there yet. Debezium's LogMiner connector needs ARCHIVELOG and minimal supplemental logging
# (documented, `docs/core/features/databases-and-change-capture.md`); the image has neither.
#
# The recovery area is capped for the CI disk (db_recovery_file_dest_size): the gate writes about
# 100,000 changes, whose archived redo is a few hundred MB.
set -eu

mode="$(sqlplus -s / as sysdba <<'SQL'
set heading off feedback off pagesize 0
SELECT log_mode FROM v$database;
SQL
)"

case "$mode" in
  *NOARCHIVELOG*)
    mkdir -p /opt/oracle/oradata/recovery_area
    sqlplus -s / as sysdba <<'SQL'
ALTER SYSTEM SET db_recovery_file_dest_size=4G SCOPE=BOTH;
ALTER SYSTEM SET db_recovery_file_dest='/opt/oracle/oradata/recovery_area' SCOPE=BOTH;
SHUTDOWN IMMEDIATE
STARTUP MOUNT
ALTER DATABASE ARCHIVELOG;
ALTER DATABASE OPEN;
ALTER PLUGGABLE DATABASE ALL OPEN;
ALTER PLUGGABLE DATABASE ALL SAVE STATE;
SQL
    ;;
esac

sqlplus -s / as sysdba <<'SQL'
ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;
SQL
