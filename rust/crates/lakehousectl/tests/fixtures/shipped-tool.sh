#!/bin/sh
# The fake shipped tool the `lakehousectl` tests run.
#
# # Why it is a committed file
#
# The tests SYMLINK this file into the install's own `tools/`
# directory, and the tool EXECUTES that link. The file the kernel
# executes is then a file nobody writes, so the kernel cannot answer
# `ETXTBSY` ("Text file busy") — what it answers when the file being
# executed still has an open write descriptor. A test binary with four
# threads can produce exactly that: another thread's child inherits
# the write descriptor of the file this thread is writing, and the
# descriptor keeps that file open for writing until the child execs
# (REL-2 S4b review round 2 SHOULD-FIX 3).
#
# Writing the fake tool and then executing it was the flake: the file
# the tool ran was the file the test had just written.
#
# # What it does
#
# The file `fake-tool-mode` in the install's own directory says what
# this tool is:
#
#   `backup`  — the default: record what it was told, and print the
#               line the tool reads back as the name of the backup;
#   `restore` — record, and say the restore is done;
#   `silent`  — record, and say nothing that names a backup;
#   `fail`    — say what went wrong on its error stream, and fail.
#
# The record it writes is read by the tests.

here="$(dirname "$0")"
mode="$(cat "$here/../fake-tool-mode" 2>/dev/null || echo backup)"
record="$here/../record.txt"

{
  echo "LAKEHOUSE_DIR=$LAKEHOUSE_DIR"
  echo "COMPOSE_PROJECT=$COMPOSE_PROJECT"
  echo "BACKUP_VERSION=$BACKUP_VERSION"
  echo "ARGS=$*"
} >> "$record"

case "$mode" in
  fail)
    echo "the volume is not there" >&2
    exit 1
    ;;
  restore)
    echo "the restore is done"
    ;;
  silent)
    echo "done"
    ;;
  *)
    echo "==> Backup complete: $LAKEHOUSE_DIR/backups/20261101T000000Z-0.2.0"
    ;;
esac
