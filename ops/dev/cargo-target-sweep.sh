#!/bin/sh
# Keep a shared cargo target directory under a byte cap (OPS-17).
#
# Cargo never removes its own old build output, so a long-lived build
# machine grows until something else cleans up. This sweeps only the
# subtrees whose contents are disposable build products, oldest mtime
# first. It measures each candidate once and subtracts its size from a
# running total, so the walk stays linear in the number of deletions,
# and re-measures the target exactly once at the end; the exit code
# follows that real size. It refuses to run under a cargo or rustc
# process: deleting objects a live build may still need would break
# that build.

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
# incremental and build. Cargo stores one directory per build script
# under build/ (`build/<crate>-<hash>/`), so its entries are listed with
# -type d exactly like incremental's, and one candidate is removed whole
# with `rm -rf --`. `find` without -L and -type f/-type d keep symlinks
# out.
collect_candidates() {
    sweep_root=$1
    for profile in "$sweep_root"/*; do
        if [ ! -d "$profile" ] || [ -L "$profile" ]; then
            continue
        fi
        if [ -d "$profile/deps" ]; then
            find "$profile/deps" -mindepth 1 -maxdepth 1 -type f -printf '%T@ %p\n'
        fi
        if [ -d "$profile/build" ]; then
            find "$profile/build" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %p\n'
        fi
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

# Measure each candidate once and subtract its size from a running
# total, instead of re-walking the whole target after every deletion:
# `du -sb` over a real target (100k+ files) once per `rm` made a full
# sweep quadratic. The loop stops as soon as the running total reaches
# the cap, and the target itself is re-measured exactly once afterwards,
# so the reported size and the exit code follow a real measurement, not
# the estimate.
while [ "$size" -gt "$cap_bytes" ]; do
    if [ -z "$candidates" ]; then
        # Nothing left to free. A real run reports the real, re-measured
        # size; a dry-run keeps the projected running total from its own
        # subtractions and never re-measures.
        if [ "$dry_run" -eq 0 ]; then
            size=$(du_size "$target_dir")
        fi
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
    entry_size=$(du_size "$path")

    if [ "$dry_run" -eq 1 ]; then
        printf 'would delete: %s\n' "$path"
    else
        rm -rf -- "$path"
    fi
    size=$((size - entry_size))
    deleted=$((deleted + 1))
done

if [ "$dry_run" -eq 1 ]; then
    printf '%s: before=%s after=%s would-delete=%s\n' "$PROGRAM" "$before" "$size" "$deleted"
else
    size=$(du_size "$target_dir")
    printf '%s: before=%s after=%s deleted=%s\n' "$PROGRAM" "$before" "$size" "$deleted"
    if [ "$size" -gt "$cap_bytes" ]; then
        # Files may have grown during the run; never claim success on the
        # estimate alone.
        exit 2
    fi
fi
