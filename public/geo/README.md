# Bundled boundary files

Map charts draw region outlines from these local GeoJSON files. Nothing is
fetched from an external tile or boundary server, so maps work offline, in
embeds, and on a self-hosted deployment with no outbound access.

| File | What | Units | Source | Licence |
|---|---|---|---|---|
| `dki-jakarta.geojson` | Jakarta, drawn per kecamatan, named by city | 42 | added in `ef7f67f`; origin not recorded | not recorded |
| `id-provinces.geojson` | Indonesia, provinces (ADM1) | 34 | OCHA ROAP / HDX `cod-ab-idn`, boundaries as of 2019, via geoBoundaries `gbHumanitarian` release `9469f09` | CC BY 3.0 IGO |
| `id-regencies.geojson` | Indonesia, kabupaten and kota (ADM2) | 519 | Badan Pusat Statistik, World Food Programme, OCHA ROAP, boundaries as of 2020, via geoBoundaries `gbOpen` release `9469f09` | CC BY 3.0 IGO |

## Known limits

- **Provinces are the 34 of 2019.** The provinces created in Papua in 2022
  are not in `id-provinces.geojson`; rows for them match no region and are
  reported as unmatched, not drawn somewhere else.
- **Simplified outlines.** The two Indonesia files were reduced from the
  geoBoundaries "simplified" release with mapshaper (`-simplify 12%` for
  provinces, `14%` for regencies, `keep-shapes`, coordinates rounded to
  0.001°, about 100 m). They are for thematic maps at national or
  provincial zoom, not for measuring or for street-level detail.
- **Names are the join key.** Each feature carries one property, `name`
  (`shapeName` upstream). A kabupaten is its bare name (`Bandung`) and a
  kota carries the prefix (`Kota Bandung`).
- The licence and origin of `dki-jakarta.geojson` were not written down
  when it was added. That is a gap to close, not a claim that it is free
  to redistribute.

## Attribution

Runfola, D. et al. (2020) geoBoundaries: A global database of political
administrative boundaries. PLoS ONE 15(4): e0231866.
https://doi.org/10.1371/journal.pone.0231866
