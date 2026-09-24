-- Seed data ClickHouse untuk stack lokal `docker-compose.yml`.
--
-- KENAPA FILE INI ADA
-- Repo memuat 20 migrasi Postgres (`rust/migrations/`), tapi TIDAK ADA satu
-- pun seed untuk ClickHouse. Akibatnya `docker compose up` menghasilkan
-- ClickHouse kosong, dan setiap halaman analitik (Catalog, Overview, Data
-- Explorer, Storage, Dashboards, Query Studio) tampil kosong atau 503 —
-- bukan karena rusak, tapi karena database `serving` dan `lake` yang dibaca
-- aplikasi memang belum pernah dibuat.
--
-- CAKUPAN
-- Skema di bawah diturunkan dari kode yang MEMBACANYA, bukan dikarang:
--   * `serving.mart_*`        <- src/lib/dashboard-specs.ts (KPIS + CHARTS)
--   * lake.`bronze_meta*.*`   <- rust/crates/lakehouse-api/src/routes/catalog.rs
--                                dan .../routes/overview.rs
--   * `silver.*`              <- catalog.rs memindai
--                                system.tables WHERE database IN ('silver','serving')
--
-- Nama tabel di `lake` memang mengandung TITIK sebagai bagian dari namanya
-- (`bronze_meta.dataset_catalog`), bukan pemisah database. Itu sebabnya
-- kode Rust mengutipnya dengan backtick, dan file ini melakukan hal sama.
--
-- SIFAT DATA
-- Angka di bawah adalah data CONTOH berbentuk realistis untuk menilai UI
-- (kepadatan tabel, layout chart, paginasi, empty state). Ini BUKAN data
-- pariwisata sungguhan dan tidak boleh dipakai untuk keputusan apa pun.
--
-- IDEMPOTEN
-- Aman dijalankan berulang: semua objek memakai `IF NOT EXISTS`, dan setiap
-- tabel di-`TRUNCATE` sebelum diisi ulang.
--
-- Pakai lewat `scripts/seed-clickhouse.sh`.

CREATE DATABASE IF NOT EXISTS serving;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS lake;

-- ---------------------------------------------------------------------------
-- serving.* — mart yang menyuplai KPI dan chart dashboard
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS serving.mart_wisman
(
    tahun       UInt16,
    bulan_no    UInt8,
    negara      String,
    kawasan     String,
    pintu_masuk String,
    jumlah      UInt32
)
ENGINE = MergeTree
ORDER BY (tahun, bulan_no, negara);

CREATE TABLE IF NOT EXISTS serving.mart_kunjungan_dtw
(
    destinasi String,
    wisnus    UInt32,
    wisman    UInt32,
    total     UInt32
)
ENGINE = MergeTree
ORDER BY destinasi;

CREATE TABLE IF NOT EXISTS serving.mart_event
(
    tahun        UInt16,
    jumlah_event UInt32
)
ENGINE = MergeTree
ORDER BY tahun;

CREATE TABLE IF NOT EXISTS serving.mart_gci_readiness
(
    indikator     String,
    readiness     String,
    data_tersedia UInt8
)
ENGINE = MergeTree
ORDER BY indikator;

CREATE TABLE IF NOT EXISTS serving.mart_kuliner
(
    wilayah      String,
    jumlah_usaha UInt32
)
ENGINE = MergeTree
ORDER BY wilayah;

CREATE TABLE IF NOT EXISTS serving.mart_atlas
(
    kategori   String,
    jumlah_poi UInt32
)
ENGINE = MergeTree
ORDER BY kategori;

-- ---------------------------------------------------------------------------
-- silver.* — lapisan antara Bronze dan Gold.
--
-- Katalog memindai `system.tables WHERE database IN ('silver','serving')`,
-- jadi tabel di sini muncul sebagai aset Silver dan membuat jenjang
-- Bronze -> Silver -> Gold terlihat utuh di UI, bukan melompat.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS silver.wisman_bulanan
(
    tahun    UInt16,
    bulan_no UInt8,
    negara   String,
    jumlah   UInt32
)
ENGINE = MergeTree
ORDER BY (tahun, bulan_no);

CREATE TABLE IF NOT EXISTS silver.dtw_kunjungan
(
    destinasi String,
    tahun     UInt16,
    wisnus    UInt32,
    wisman    UInt32
)
ENGINE = MergeTree
ORDER BY (destinasi, tahun);

CREATE TABLE IF NOT EXISTS silver.event_tahunan
(
    tahun      UInt16,
    nama_event String,
    lokasi     String
)
ENGINE = MergeTree
ORDER BY tahun;

-- ---------------------------------------------------------------------------
-- lake.* — metadata katalog dataset.
--
-- Nama tabel memuat titik sebagai bagian dari namanya; backtick wajib.
-- `bronze_meta` = dataset primer, `bronze_meta_sec` = sekunder. Keduanya
-- di-UNION oleh catalog.rs, dan kolom `tier` menentukan namespace yang
-- tampil di UI (`sdi-primer` vs `sekunder`).
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS lake.`bronze_meta.dataset_catalog`
(
    slug        String,
    title       String,
    description String,
    tier        String,
    updated_at  String,
    table_name  String
)
ENGINE = MergeTree
ORDER BY slug;

CREATE TABLE IF NOT EXISTS lake.`bronze_meta.dataset_sync`
(
    slug        String,
    title       String,
    description String,
    table_name  String,
    total       UInt64,
    author      String,
    frekuensi   String,
    satuan      String,
    klasifikasi String
)
ENGINE = MergeTree
ORDER BY slug;

CREATE TABLE IF NOT EXISTS lake.`bronze_meta.dataset_column`
(
    slug      String,
    key_asli  String,
    tipe      String,
    deskripsi String
)
ENGINE = MergeTree
ORDER BY (slug, key_asli);

CREATE TABLE IF NOT EXISTS lake.`bronze_meta_sec.dataset_catalog`
(
    slug        String,
    title       String,
    description String,
    tier        String,
    updated_at  String,
    table_name  String
)
ENGINE = MergeTree
ORDER BY slug;

CREATE TABLE IF NOT EXISTS lake.`bronze_meta_sec.dataset_sync`
(
    slug        String,
    title       String,
    description String,
    table_name  String,
    total       UInt64,
    author      String,
    frekuensi   String,
    satuan      String,
    klasifikasi String
)
ENGINE = MergeTree
ORDER BY slug;

CREATE TABLE IF NOT EXISTS lake.`bronze_meta_sec.dataset_column`
(
    slug      String,
    key_asli  String,
    tipe      String,
    deskripsi String
)
ENGINE = MergeTree
ORDER BY (slug, key_asli);

-- ===========================================================================
-- ISI DATA
-- Setiap tabel dikosongkan lebih dulu supaya file ini aman dijalankan ulang.
-- ===========================================================================

TRUNCATE TABLE serving.mart_wisman;

-- Kunjungan wisatawan mancanegara 2020-2024.
--
-- Dibangkitkan sebagai perkalian tahun x bulan x negara supaya chart tren
-- punya deret waktu yang benar-benar bergerak, bukan garis datar. Bentuk
-- angkanya sengaja meniru pola nyata: anjlok di 2020-2021 (pandemi), pulih
-- bertahap 2022-2024, dengan puncak di pertengahan dan akhir tahun.
INSERT INTO serving.mart_wisman (tahun, bulan_no, negara, kawasan, pintu_masuk, jumlah)
SELECT
    tahun,
    bulan_no,
    negara,
    kawasan,
    pintu_masuk,
    toUInt32(greatest(
        50,
        round(
            basis
            -- Pemulihan pascapandemi: 2020 terpuruk, naik bertahap ke 2024.
            * multiIf(tahun = 2020, 0.22,
                      tahun = 2021, 0.35,
                      tahun = 2022, 0.68,
                      tahun = 2023, 0.89,
                      1.0)
            -- Musiman: naik sekitar Juli dan Desember.
            * (1 + 0.35 * sin((bulan_no - 3) * pi() / 6))
            -- Variasi kecil supaya angka tidak terlihat dibuat mesin.
            * (0.85 + 0.3 * (cityHash64(tahun, bulan_no, negara) % 100) / 100)
        )
    )) AS jumlah
FROM
(
    SELECT
        arrayJoin([2020, 2021, 2022, 2023, 2024]) AS tahun,
        arrayJoin(range(1, 13))                   AS bulan_no
) AS periode
CROSS JOIN
(
    SELECT
        tupleElement(n, 1) AS negara,
        tupleElement(n, 2) AS kawasan,
        tupleElement(n, 3) AS pintu_masuk,
        tupleElement(n, 4) AS basis
    FROM
    (
        SELECT arrayJoin([
            ('Malaysia',      'ASEAN',        'Bandara Soekarno-Hatta', 42000),
            ('Singapura',     'ASEAN',        'Bandara Ngurah Rai',     38000),
            ('Australia',     'Oseania',      'Bandara Ngurah Rai',     31000),
            ('Tiongkok',      'Asia Timur',   'Bandara Soekarno-Hatta', 27000),
            ('Jepang',        'Asia Timur',   'Bandara Ngurah Rai',     18000),
            ('Korea Selatan', 'Asia Timur',   'Bandara Soekarno-Hatta', 15000),
            ('India',         'Asia Selatan', 'Bandara Ngurah Rai',     13000),
            ('Amerika Serikat','Amerika',     'Bandara Soekarno-Hatta', 11000),
            ('Belanda',       'Eropa',        'Pelabuhan Batam',         8000),
            ('Jerman',        'Eropa',        'Bandara Ngurah Rai',      7500),
            ('Inggris',       'Eropa',        'Bandara Juanda',          6800),
            ('Prancis',       'Eropa',        'Pelabuhan Batam',         5200)
        ]) AS n
    )
) AS negara_ref;

TRUNCATE TABLE serving.mart_kunjungan_dtw;

-- Destinasi wisata (DTW). `total` sengaja disimpan, bukan dihitung ulang,
-- karena chart mengurutkan dengan `ORDER BY sum(total) DESC`.
INSERT INTO serving.mart_kunjungan_dtw (destinasi, wisnus, wisman) VALUES
    ('Candi Borobudur',        2840000, 412000),
    ('Pantai Kuta',            1960000, 738000),
    ('Taman Mini Indonesia',   1720000,  64000),
    ('Candi Prambanan',        1480000, 231000),
    ('Kawah Ijen',              890000, 142000),
    ('Danau Toba',              820000,  58000),
    ('Tanah Lot',               760000, 394000),
    ('Bromo Tengger Semeru',    740000, 168000),
    ('Raja Ampat',              186000,  92000),
    ('Pulau Komodo',            164000, 121000),
    ('Malioboro',             1340000,  87000),
    ('Ubud Monkey Forest',      420000, 286000),
    ('Kepulauan Seribu',        512000,  34000),
    ('Tangkuban Perahu',        648000,  27000),
    ('Pantai Parangtritis',     712000,  19000);

-- Sinkronkan kolom turunan agar konsisten dengan komponennya.
ALTER TABLE serving.mart_kunjungan_dtw UPDATE total = wisnus + wisman WHERE 1;

TRUNCATE TABLE serving.mart_event;

INSERT INTO serving.mart_event (tahun, jumlah_event) VALUES
    (2019, 284),
    (2020,  62),
    (2021,  98),
    (2022, 217),
    (2023, 341),
    (2024, 396);

TRUNCATE TABLE serving.mart_gci_readiness;

-- Kesiapan data indikator Global Competitiveness Index.
-- `data_tersedia` (0/1) dijumlahkan untuk KPI; `readiness` dikelompokkan
-- untuk chart pie.
INSERT INTO serving.mart_gci_readiness (indikator, readiness, data_tersedia) VALUES
    ('Konektivitas Udara',            'Siap',           1),
    ('Infrastruktur Jalan',           'Siap',           1),
    ('Kesehatan dan Higienitas',      'Siap',           1),
    ('Keamanan dan Keselamatan',      'Siap',           1),
    ('Kualitas SDM Pariwisata',       'Siap',           1),
    ('Daya Saing Harga',              'Siap',           1),
    ('Kesiapan Teknologi Informasi',  'Sebagian',       1),
    ('Keberlanjutan Lingkungan',      'Sebagian',       1),
    ('Sumber Daya Budaya',            'Sebagian',       1),
    ('Keterbukaan Internasional',     'Belum tersedia', 0),
    ('Prioritas Perjalanan',          'Belum tersedia', 0),
    ('Sumber Daya Alam',              'Belum tersedia', 0);

TRUNCATE TABLE serving.mart_kuliner;

INSERT INTO serving.mart_kuliner (wilayah, jumlah_usaha) VALUES
    ('DKI Jakarta',      18420),
    ('Jawa Barat',       16780),
    ('Jawa Timur',       14260),
    ('Jawa Tengah',      12940),
    ('Bali',              9860),
    ('Sumatera Utara',    7320),
    ('Banten',            6180),
    ('DI Yogyakarta',     5740),
    ('Sulawesi Selatan',  4920),
    ('Sumatera Barat',    3860);

TRUNCATE TABLE serving.mart_atlas;

INSERT INTO serving.mart_atlas (kategori, jumlah_poi) VALUES
    ('Wisata Alam',      3842),
    ('Wisata Budaya',    2916),
    ('Wisata Buatan',    1764),
    ('Kuliner',          1482),
    ('Belanja',           936),
    ('Religi',            824),
    ('Sejarah',           712),
    ('Olahraga dan Rekreasi', 458);

-- ---------------------------------------------------------------------------
-- silver.* — diturunkan dari mart supaya angka antar lapisan tidak
-- bertentangan saat ditelusuri lewat lineage di UI.
-- ---------------------------------------------------------------------------

TRUNCATE TABLE silver.wisman_bulanan;
INSERT INTO silver.wisman_bulanan (tahun, bulan_no, negara, jumlah)
SELECT tahun, bulan_no, negara, jumlah FROM serving.mart_wisman;

TRUNCATE TABLE silver.dtw_kunjungan;
INSERT INTO silver.dtw_kunjungan (destinasi, tahun, wisnus, wisman)
SELECT destinasi, 2024, wisnus, wisman FROM serving.mart_kunjungan_dtw;

TRUNCATE TABLE silver.event_tahunan;
INSERT INTO silver.event_tahunan (tahun, nama_event, lokasi) VALUES
    (2024, 'Java Jazz Festival',     'DKI Jakarta'),
    (2024, 'Bali Arts Festival',     'Bali'),
    (2024, 'Festival Danau Toba',    'Sumatera Utara'),
    (2024, 'Jogja Culinary Week',    'DI Yogyakarta'),
    (2023, 'Borobudur Marathon',     'Jawa Tengah'),
    (2023, 'Festival Krakatau',      'Lampung'),
    (2023, 'Tour de Singkarak',      'Sumatera Barat'),
    (2022, 'Dieng Culture Festival', 'Jawa Tengah');

-- ---------------------------------------------------------------------------
-- lake.* — katalog dataset yang menyuplai halaman Catalog, Data Explorer,
-- dan kartu ringkasan di Overview.
--
-- `slug` adalah kunci penghubung antar ketiga tabel; `table_name` menunjuk ke
-- tabel fisik sehingga catalog.rs bisa mencocokkannya dengan `system.tables`.
-- ---------------------------------------------------------------------------

TRUNCATE TABLE lake.`bronze_meta.dataset_catalog`;
INSERT INTO lake.`bronze_meta.dataset_catalog` (slug, title, description, tier, updated_at, table_name) VALUES
    ('wisman-bulanan',   'Kunjungan Wisatawan Mancanegara', 'Jumlah kunjungan wisman per bulan menurut negara asal, kawasan, dan pintu masuk.', 'primer', '2026-09-01T08:00:00Z', 'mart_wisman'),
    ('kunjungan-dtw',    'Kunjungan Daya Tarik Wisata',     'Kunjungan wisatawan nusantara dan mancanegara per destinasi.',                     'primer', '2026-09-01T08:00:00Z', 'mart_kunjungan_dtw'),
    ('event-pariwisata', 'Event Pariwisata Tahunan',        'Jumlah event pariwisata yang terselenggara per tahun.',                            'primer', '2026-08-28T10:30:00Z', 'mart_event'),
    ('gci-readiness',    'Kesiapan Data Indikator GCI',     'Status ketersediaan data untuk indikator Global Competitiveness Index.',           'primer', '2026-08-25T14:15:00Z', 'mart_gci_readiness');

TRUNCATE TABLE lake.`bronze_meta.dataset_sync`;
INSERT INTO lake.`bronze_meta.dataset_sync` (slug, title, description, table_name, total, author, frekuensi, satuan, klasifikasi) VALUES
    ('wisman-bulanan',   'Kunjungan Wisatawan Mancanegara', 'Kunjungan wisman per bulan.',             'mart_wisman',        720, 'Badan Pusat Statistik',     'Bulanan',  'kunjungan', 'Terbuka'),
    ('kunjungan-dtw',    'Kunjungan Daya Tarik Wisata',     'Kunjungan per destinasi.',                'mart_kunjungan_dtw',  15, 'Dinas Pariwisata Provinsi', 'Tahunan',  'kunjungan', 'Terbuka'),
    ('event-pariwisata', 'Event Pariwisata Tahunan',        'Jumlah event per tahun.',                 'mart_event',           6, 'Kementerian Pariwisata',    'Tahunan',  'event',     'Terbuka'),
    ('gci-readiness',    'Kesiapan Data Indikator GCI',     'Status ketersediaan data indikator GCI.', 'mart_gci_readiness',  12, 'Tim Data Pariwisata',       'Triwulan', 'indikator', 'Internal');

TRUNCATE TABLE lake.`bronze_meta.dataset_column`;
INSERT INTO lake.`bronze_meta.dataset_column` (slug, key_asli, tipe, deskripsi) VALUES
    ('wisman-bulanan',   'tahun',         'UInt16', 'Tahun kunjungan'),
    ('wisman-bulanan',   'bulan_no',      'UInt8',  'Nomor bulan (1-12)'),
    ('wisman-bulanan',   'negara',        'String', 'Negara asal wisatawan'),
    ('wisman-bulanan',   'kawasan',       'String', 'Kawasan/benua asal'),
    ('wisman-bulanan',   'pintu_masuk',   'String', 'Pintu masuk kedatangan'),
    ('wisman-bulanan',   'jumlah',        'UInt32', 'Jumlah kunjungan'),
    ('kunjungan-dtw',    'destinasi',     'String', 'Nama daya tarik wisata'),
    ('kunjungan-dtw',    'wisnus',        'UInt32', 'Kunjungan wisatawan nusantara'),
    ('kunjungan-dtw',    'wisman',        'UInt32', 'Kunjungan wisatawan mancanegara'),
    ('kunjungan-dtw',    'total',         'UInt32', 'Total kunjungan'),
    ('event-pariwisata', 'tahun',         'UInt16', 'Tahun penyelenggaraan'),
    ('event-pariwisata', 'jumlah_event',  'UInt32', 'Jumlah event'),
    ('gci-readiness',    'indikator',     'String', 'Nama indikator GCI'),
    ('gci-readiness',    'readiness',     'String', 'Status kesiapan data'),
    ('gci-readiness',    'data_tersedia', 'UInt8',  'Penanda ketersediaan data (0/1)');

TRUNCATE TABLE lake.`bronze_meta_sec.dataset_catalog`;
INSERT INTO lake.`bronze_meta_sec.dataset_catalog` (slug, title, description, tier, updated_at, table_name) VALUES
    ('usaha-kuliner', 'Usaha Kuliner Terdaftar', 'Jumlah usaha kuliner terdaftar per wilayah.',            'sekunder', '2026-08-20T09:00:00Z', 'mart_kuliner'),
    ('atlas-poi',     'Atlas Titik Wisata',      'Jumlah titik daya tarik wisata (POI) menurut kategori.', 'sekunder', '2026-08-18T16:45:00Z', 'mart_atlas');

TRUNCATE TABLE lake.`bronze_meta_sec.dataset_sync`;
INSERT INTO lake.`bronze_meta_sec.dataset_sync` (slug, title, description, table_name, total, author, frekuensi, satuan, klasifikasi) VALUES
    ('usaha-kuliner', 'Usaha Kuliner Terdaftar', 'Usaha kuliner per wilayah.', 'mart_kuliner', 10, 'Dinas Koperasi dan UKM',     'Tahunan', 'usaha', 'Terbuka'),
    ('atlas-poi',     'Atlas Titik Wisata',      'POI menurut kategori.',      'mart_atlas',    8, 'Badan Informasi Geospasial', 'Tahunan', 'titik', 'Terbuka');

TRUNCATE TABLE lake.`bronze_meta_sec.dataset_column`;
INSERT INTO lake.`bronze_meta_sec.dataset_column` (slug, key_asli, tipe, deskripsi) VALUES
    ('usaha-kuliner', 'wilayah',      'String', 'Nama provinsi'),
    ('usaha-kuliner', 'jumlah_usaha', 'UInt32', 'Jumlah usaha kuliner terdaftar'),
    ('atlas-poi',     'kategori',     'String', 'Kategori daya tarik wisata'),
    ('atlas-poi',     'jumlah_poi',   'UInt32', 'Jumlah titik POI');
