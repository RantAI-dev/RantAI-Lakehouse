#!/usr/bin/env bash
# Prepare a self-hosted CI worker's build directory (OPS-24 D10).
#
# Runs only on a self-hosted runner, after checkout and after the job has
# refused a runner that is not the CI worker. It prints the machine's facts,
# then gives cargo a target directory that survives between runs, bounded in
# size, and exports it through $GITHUB_ENV.
#
# Inputs (environment):
#   RUNNER_NAME        the runner registration's name (one directory per
#                      registration)
#   HOME               the runner user's home
#   GITHUB_WORKSPACE   the checkout
#   WORKER_CACHE_ROOT  optional; default $HOME/.cache/lakehouse-ci
#   TARGET_MAX_GB      optional; the directory's limit in GiB, 5 to 500,
#                      default 40
#   WORKER_CARGO_JOBS  optional; CARGO_BUILD_JOBS, 1 to 64
#   GITHUB_ENV         where CARGO_TARGET_DIR and CARGO_BUILD_JOBS go
#
# The bound deletes only this runner's own cache: files not read for three
# days, then -- if that was not enough -- the directory itself, which is
# recreated empty. Never `cargo clean`, never on every run.
set -euo pipefail

# Runner facts. None of these may fail the job: they are what the owner reads
# to size the machine. No host name and no address is printed; the runner's
# registration name is not one.
echo "runner=${RUNNER_NAME:-}" || true
nproc || true
free -g || true
df -h "${HOME:-/}" "${GITHUB_WORKSPACE:-}" || true
findmnt -no OPTIONS -T "${HOME:-/}" || true
docker info --format 'docker={{.ServerVersion}} cpus={{.NCPU}} mem={{.MemTotal}} containers={{.Containers}} images={{.Images}}' || true
rustup show || true
ldd --version | head -1 || true

RUNNER="${RUNNER_NAME:-}"
if [ -z "$RUNNER" ]; then
  echo "::error::RUNNER_NAME is empty; the cache needs one directory per runner registration"
  exit 1
fi

# One directory per registration: two registrations never build in the same
# directory, and cargo's own lock on the shared ~/.cargo registry covers the
# downloads.
name="$(printf '%s' "$RUNNER" | sed 's/[^A-Za-z0-9._-]/_/g')"
root="${WORKER_CACHE_ROOT:-${HOME}/.cache/lakehouse-ci}"
dir="${root}/target/${name}"
mkdir -p "$dir"

limit_gb=40
if [[ "${TARGET_MAX_GB:-}" =~ ^[0-9]+$ ]]; then
  candidate=$((10#${TARGET_MAX_GB}))
  if [ "$candidate" -ge 5 ] && [ "$candidate" -le 500 ]; then
    limit_gb="$candidate"
  fi
fi
limit_kb=$((limit_gb * 1024 * 1024))

used_kb="$(du -sk "$dir" | awk '{print $1}')"
echo "worker cache: ${dir} is ${used_kb} KiB, limit ${limit_kb} KiB (${limit_gb} GiB)"

if [ "$used_kb" -gt "$limit_kb" ]; then
  echo "worker cache: over the limit; stage 1 deletes files not read for 3 days"
  find "$dir" -type f -atime +3 -delete
  used_kb="$(du -sk "$dir" | awk '{print $1}')"
  echo "worker cache: ${used_kb} KiB after stage 1"
  if [ "$used_kb" -gt "$limit_kb" ]; then
    case "$dir" in
      "${root}/target/"*)
        echo "::warning::worker cache: still over the limit; stage 2 removes ${dir} and recreates it"
        rm -rf -- "$dir"
        mkdir -p "$dir"
        echo "worker cache: ${dir} was recreated empty"
        ;;
      *)
        echo "::error::refusing to remove ${dir}: it is not under ${root}/target/"
        exit 1
        ;;
    esac
  fi
fi

if [ -n "${GITHUB_ENV:-}" ]; then
  echo "CARGO_TARGET_DIR=${dir}" >> "$GITHUB_ENV"
else
  echo "CARGO_TARGET_DIR=${dir}"
fi

if [[ "${WORKER_CARGO_JOBS:-}" =~ ^[0-9]+$ ]]; then
  jobs=$((10#${WORKER_CARGO_JOBS}))
  if [ "$jobs" -ge 1 ] && [ "$jobs" -le 64 ]; then
    if [ -n "${GITHUB_ENV:-}" ]; then
      echo "CARGO_BUILD_JOBS=${jobs}" >> "$GITHUB_ENV"
    else
      echo "CARGO_BUILD_JOBS=${jobs}"
    fi
  fi
fi
