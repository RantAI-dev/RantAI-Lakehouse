/**
 * Identitas tenant yang dipakai route server saat melabeli aset, audit, dan
 * kuota. Dulu nilai-nilai ini ditulis langsung di `src/app/api/**` (mengunci
 * konsol ke satu instalasi). Sekarang dibaca dari environment supaya satu image
 * bisa melayani deployment berbeda — produksi Dispar maupun demo mitra.
 *
 * Semua punya default lama, jadi deployment yang tidak menyetel apa pun
 * berperilaku persis seperti sebelumnya.
 */

/** Nama organisasi pemilik data — tampil sebagai `owner` aset katalog. */
export const TENANT_OWNER =
  process.env.TENANT_OWNER ?? "Dinas Pariwisata & Ekraf DKI Jakarta";

/** ID tenant untuk audit, kuota, dan residensi. */
export const TENANT_ID = process.env.TENANT_ID ?? "dispar-dki";

/** Domain bisnis default aset katalog. */
export const TENANT_DOMAIN = process.env.TENANT_DOMAIN ?? "pariwisata";

/** Label residensi data (di mana data boleh berada). */
export const TENANT_RESIDENCY = process.env.TENANT_RESIDENCY ?? "id-jakarta";

/** Lokasi fisik/site tempat layanan berjalan. */
export const TENANT_SITE = process.env.TENANT_SITE ?? "Depok (187)";

/**
 * Slug dataset yang dianggap Bronze TERKURASI (bukan sekadar landing raw).
 * Dataset di luar daftar ini muncul sebagai layer `raw` di katalog.
 *
 * Default-nya daftar Dispar supaya deployment lama tak berubah perilaku; isi
 * `BRONZE_CURATED_SLUGS` (dipisah koma) untuk deployment lain.
 */
/** Label namespace katalog (`primer`, `sekunder`, `silver`, `serving`). */
export type NamespaceMeta = { name: string; description: string };

/**
 * ID namespace Bronze sumber primer. Dulu `"sdi-primer"` — "SDI" itu
 * kosakata program Satu Data Indonesia milik satu deployment, tapi tampil
 * di SELURUH aset deployment lain. ID-nya kini generik (lihat
 * `tenant::NAMESPACE_PRIMER` di sisi Rust, yang memancarkan nilai ini).
 */
const NAMESPACE_META_DEFAULT: Record<string, NamespaceMeta> = {
  primer: {
    name: "Sumber Primer",
    description: "Dataset primer dari sistem sumber tenant, mendarat di Bronze/Iceberg.",
  },
  sekunder: {
    name: "Sumber Sekunder (olahan)",
    description: "Dataset sekunder: umpan pihak ketiga dan ekstrak olahan.",
  },
  silver: {
    name: "Silver (kurasi)",
    description: "Model bersih & terkonform di ClickHouse — dimensi dan fakta.",
  },
  serving: {
    name: "Gold (mart penyaji)",
    description: "Mart agregat penyaji dashboard dan embed.",
  },
};

/**
 * Label namespace yang dipakai katalog. Isi `CATALOG_NAMESPACE_META` dengan
 * objek JSON `{ "<id>": { "name": ..., "description": ... } }` untuk menimpa
 * sebagian atau seluruhnya; JSON rusak diabaikan (jatuh ke default).
 */
export const NAMESPACE_META: Record<string, NamespaceMeta> = (() => {
  const raw = process.env.CATALOG_NAMESPACE_META;
  if (!raw) return NAMESPACE_META_DEFAULT;
  try {
    const parsed = JSON.parse(raw) as Record<string, NamespaceMeta>;
    return { ...NAMESPACE_META_DEFAULT, ...parsed };
  } catch {
    return NAMESPACE_META_DEFAULT;
  }
})();

export const BRONZE_CURATED_SLUGS: ReadonlySet<string> = new Set(
  (
    process.env.BRONZE_CURATED_SLUGS ??
    "wisman-jakarta-per-bulan,wisman-jakarta-per-negara,wisman-jakarta-per-pintu-masuk,jumlah-pengunjung-event-2026"
  )
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean),
);
