# Aset merek — Rantai Lake design kit

## Yang dipakai aplikasi

| Aset | Lokasi di app | Keterangan |
|------|----------------|------------|
| Logo raster | `public/rantai-logo.png` | Dipakai di `AppSidebar` (`/rantai-logo.png`). Jika file belum ada di repo Anda, ekspor dari Figma atau tim brand dan letakkan di `public/`. |

## Yang disediakan di folder ini

| File | Keterangan |
|------|------------|
| `rantai-logo.png` | Salinan logo sidebar dari `public/rantai-logo.png` — bawa ke proyek baru dengan menyalin file ini ke `public/rantai-logo.png`. |
| `rantai-mark.svg` | Mark vektor sederhana (kotak orange + teks "NQ") untuk **placeholder dev** atau favicon sementara. **Bukan** pengganti resmi logo jika brand punya file final. |

## Menambahkan ke proyek baru

1. Salin `rantai-mark.svg` ke `public/` jika butuh ikon cepat.
2. Salin `rantai-logo.png` resmi ke `public/rantai-logo.png` agar sidebar konsisten dengan Rantai Lake.
3. Untuk favicon Next.js, letakkan `favicon.ico` / `icon.png` di `src/app/` atau `public/` sesuai dokumentasi Next.js 16.

## Warna mark (referensi)

- Isian utama mark placeholder: **#ff5001** (sama dengan `--primary` light).
