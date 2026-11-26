#!/bin/sh
# Keep a shared cargo target directory under a byte cap (OPS-17).
#
# Cargo never removes its own old build output, so a long-lived build
# machine grows until something else cleans up. This sweeps only the
# subtrees whose contents are disposable build products, oldest mtime
# first, and stops as soon as `du -sb` reports a size at or under the cap.
# It refuses to run under a cargo or rustc process: deleting objects a
# live build may still need would break that build.

set -eu

PROGRAM=${0##*/}

usage() {
    printf 'usage: %s [--dry-run] <target-dir> <cap>\n' "$PROGRAM" >&2
}

fail_usage() {
    printf '%s: %s\n' "$PROGRAM" "$1" >&2
    usage
    exit 1
}

dry_run=0
if [ "${1-}" = "--dry-run" ]; then
    dry_run=1
    shift
fi

if [ "$#" -ne 2 ]; then
    fail_usage "expected a target directory and a cap"
fi
target_dir=$1
cap_arg=$2

if [ ! -d "$target_dir" ]; then
    fail_usage "not a directory: $target_dir"
fi
if [ ! -r "$target_dir" ] || [ ! -x "$target_dir" ]; then
    fail_usage "cannot read directory: $target_dir"
fi

# A cap is digits with an optional K/M/G/T suffix; anything else is a
# usage error rather than a silent misread of the operator's intent.
case "$cap_arg" in
    ''|*[!0-9A-Z]*) fail_usage "invalid cap: $cap_arg" ;;
esac
case "$cap_arg" in
    *[KMGT])
        number_part=${cap_arg%?}
        suffix=${cap_arg#"$number_part"}
        ;;
    *)
        number_part=$cap_arg
        suffix=B
        ;;
esac
case "$number_part" in
    ''|*[!0-9]*) fail_usage "invalid cap: $cap_arg" ;;
esac
# Strip leading zeros so shell arithmetic never reads a number as octal.
while [ "$number_part" != 0 ] && [ "${number_part#0}" != "$number_part" ]; do
    number_part=${number_part#0}
done
case "$suffix" in
    B) multiplier=1 ;;
    K) multiplier=1024 ;;
    M) multiplier=1048576 ;;
    G) multiplier=1073741824 ;;
    T) multiplier=1099511627776 ;;
    *) fail_usage "invalid cap: $cap_arg" ;;
esac
cap_bytes=$((number_part * multiplier))

# A skipped run must not look like a broken unit to the scheduler.
proc_root=${SWEEP_PROC_ROOT:-/proc}
for comm_file in "$proc_root"/[0-9]*/comm; do
    if [ ! -f "$comm_file" ]; then
        continue
    fi
    if ! IFS= read -r running_name < "$comm_file"; then
        continue
    fi
    case "$running_name" in
        cargo|rustc)
            printf '%s: a %s process is running; not sweeping\n' "$PROGRAM" "$running_name"
            exit 75
            ;;
    esac
done

du_size() {
    du_line=$(du -sb "$1")
    printf '%s' "${du_line%%[[:space:]]*}"
}

# One candidate per disposable file, or per whole directory under
# incremental. `find` without -L and -type f/-type d keep symlinks out.
collect_candidates() {
    sweep_root=$1
    for profile in "$sweep_root"/*; do
        if [ ! -d "$profile" ] || [ -L "$profile" ]; then
            continue
        fi
        for subdirectory in deps build; do
            if [ -d "$profile/$subdirectory" ]; then
                find "$profile/$subdirectory" -mindepth 1 -maxdepth 1 -type f -printf '%T@ %p\n'
            fi
        done
        if [ -d "$profile/incremental" ]; then
            find "$profile/incremental" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %p\n'
        fi
    done
    if [ -d "$sweep_root/tmp" ]; then
        find "$sweep_root/tmp" -mindepth 1 -maxdepth 1 -type f -printf '%T@ %p\n'
    fi
}

candidates=$(collect_candidates "$target_dir" | LC_ALL=C sort -n)
before=$(du_size "$target_dir")
size=$before
deleted=0
newline='
'

while [ "$size" -gt "$cap_bytes" ]; do
    if [ -z "$candidates" ]; then
        printf '%s: cannot reach cap %s bytes: final size %s bytes\n' "$PROGRAM" "$cap_bytes" "$size"
        printf '%s: before=%s after=%s deleted=%s\n' "$PROGRAM" "$before" "$size" "$deleted"
        exit 2
    fi

    case "$candidates" in
        *"$newline"*)
            first=${candidates%%"$newline"*}
            candidates=${candidates#*"$newline"}
            ;;
        *)
            first=$candidates
            candidates=
            ;;
    esac
    path=${first#* }

    if [ "$dry_run" -eq 1 ]; then
        entry_size=$(du_size "$path")
        size=$((size - entry_size))
        printf 'would delete: %s\n' "$path"
    else
        rm -rf -- "$path"
        size=$(du_size "$target_dir")
    fi
    deleted=$((deleted + 1))
done

if [ "$dry_run" -eq 1 ]; then
    printf '%s: before=%s after=%s would-delete=%s\n' "$PROGRAM" "$before" "$size" "$deleted"
else
    printf '%s: before=%s after=%s deleted=%s\n' "$PROGRAM" "$before" "$size" "$deleted"
fi
