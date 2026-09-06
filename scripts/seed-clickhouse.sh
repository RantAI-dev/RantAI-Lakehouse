#!/usr/bin/env bash
# Isi ClickHouse lokal dengan data contoh dari `scripts/seed-clickhouse.sql`.
#
# Repo memuat 20 migrasi Postgres yang berjalan otomatis saat container API
# start (lihat `rust/entrypoint.api.sh`), tapi tidak ada padanannya untuk
# ClickHouse. Tanpa skrip ini, `docker compose up` menghasilkan ClickHouse
# kosong dan setiap halaman analitik tampil tanpa data.
#
# Usage:
#   scripts/seed-clickhouse.sh
#
# Env overrides (defaults match docker-compose.yml / .env.example):
#   COMPOSE_PROJECT     nama project compose (pass -p bila bukan default)
#   CLICKHOUSE_SERVICE  nama service compose (default: clickhouse)
#   CH_USER             user ClickHouse (default: default)
#   CH_PASSWORD         password ClickHouse (default: kosong)
#   SEED_FILE           file SQL yang dijalankan (default: skrip di sebelahnya)
#
# Aman dijalankan berulang: seluruh objek dibuat dengan `IF NOT EXISTS` dan
# setiap tabel di-TRUNCATE sebelum diisi ulang.
set -euo pipefail

CLICKHOUSE_SERVICE="${CLICKHOUSE_SERVICE:-clickhouse}"
CH_USER="${CH_USER:-default}"
CH_PASSWORD="${CH_PASSWORD:-}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SEED_FILE="${SEED_FILE:-$SCRIPT_DIR/seed-clickhouse.sql}"

if [ ! -f "$SEED_FILE" ]; then
    echo "seed-clickhouse: file SQL tidak ditemukan: $SEED_FILE" >&2
    exit 1
fi

# `docker compose` dijalankan dari root repo supaya menemukan
# docker-compose.yml dan .env tanpa bergantung pada cwd pemanggil.
cd "$SCRIPT_DIR/.."

# Dibungkus fungsi, bukan array: bash 3.2 (bawaan macOS) memperlakukan
# ekspansi array kosong sebagai unbound variable di bawah `set -u`.
dc() {
    if [ -n "${COMPOSE_PROJECT:-}" ]; then
        docker compose -p "$COMPOSE_PROJECT" "$@"
    else
        docker compose "$@"
    fi
}

if ! dc ps --status running --services 2>/dev/null |
    grep -qx "$CLICKHOUSE_SERVICE"; then
    echo "seed-clickhouse: service '$CLICKHOUSE_SERVICE' tidak berjalan." >&2
    echo "                 Jalankan 'docker compose up -d' lebih dulu." >&2
    exit 1
fi

# `clickhouse-client` dipakai alih-alih antarmuka HTTP karena file ini berisi
# banyak pernyataan; `--multiquery` menjalankan semuanya dalam satu koneksi,
# sedangkan endpoint HTTP hanya menerima satu query per request.
#
# `-T` mematikan alokasi TTY supaya redirect stdin tetap bekerja saat skrip
# dijalankan dari CI atau shell non-interaktif.
ch_client() {
    if [ -n "$CH_PASSWORD" ]; then
        dc exec -T "$CLICKHOUSE_SERVICE" \
            clickhouse-client --user "$CH_USER" --password "$CH_PASSWORD" "$@"
    else
        dc exec -T "$CLICKHOUSE_SERVICE" \
            clickhouse-client --user "$CH_USER" "$@"
    fi
}

echo "seed-clickhouse: menjalankan $(basename "$SEED_FILE")..." >&2
ch_client --multiquery < "$SEED_FILE"

echo "seed-clickhouse: selesai. Ringkasan baris:" >&2
ch_client --query "
        SELECT concat(database, '.', table) AS tabel, sum(rows) AS baris
        FROM system.parts
        WHERE active AND database IN ('serving', 'silver', 'lake')
        GROUP BY database, table
        ORDER BY database, table
        FORMAT PrettyCompactMonoBlock"
