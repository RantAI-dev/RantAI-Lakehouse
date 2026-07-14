# Rantai Lake — Design kit

Folder ini menyimpan **token warna/tipografi**, **aset merek**, dan **dokumentasi konteks** agar proyek baru (atau repo lain) bisa meniru tampilan dan pola **Rantai Lake** secara konsisten.

## Isi

| Path | Keterangan |
|------|------------|
| [DESIGN_CONTEXT.md](./DESIGN_CONTEXT.md) | Panduan lengkap: stack, library UI, struktur folder, checklist bootstrap proyek baru |
| [AI_CONTEXT.md](./AI_CONTEXT.md) | Ringkasan singkat untuk disalin ke Cursor / agen AI |
| [package.versions.reference.json](./package.versions.reference.json) | Versi dependency referensi dari repo ini |
| [tokens/rantai-lake-theme.css](./tokens/rantai-lake-theme.css) | Fragment CSS (Tailwind v4 + variabel shadcn) untuk disisipkan ke `globals.css` |
| [components/patterns.md](./components/patterns.md) | Pola komposisi UI yang dipakai di app ini |
| [assets/](./assets/) | Aset vektor fallback + catatan aset raster |

## Cara pakai singkat

1. Baca **DESIGN_CONTEXT.md** untuk urutan setup Next.js + shadcn + font.
2. Gabungkan isi **tokens/rantai-lake-theme.css** ke `globals.css` proyek baru (setelah import Tailwind / shadcn), lalu sesuaikan `layout` untuk font Inter + Montserrat.
3. Salin **assets** yang diperlukan ke `public/` proyek baru.
4. Install komponen shadcn lewat CLI dengan gaya **base-nova** (lihat `components.json` di repo utama).

Repo aplikasi utama tetap memakai sumber di `src/`; **design-kit** adalah salinan referensi yang bisa di-copy ke proyek lain tanpa mengubah perilaku build app saat ini.
