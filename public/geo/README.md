# Bundled boundary files

Map charts draw region outlines from these local GeoJSON files. Nothing is
fetched from an external tile or boundary server, so maps work offline, in
embeds, and on a self-hosted deployment with no outbound access.

| File | What | Units | Source | Licence |
|---|---|---|---|---|
| `dki-jakarta.geojson` | Jakarta, drawn per kecamatan, named by city | 42 | added in `ef7f67f`; origin not recorded | not recorded |
| `id-provinces.geojson` | Indonesia, provinces (ADM1) | 34 | OCHA ROAP / HDX `cod-ab-idn`, boundaries as of 2019, via geoBoundaries `gbHumanitarian` release `9469f09` | CC BY 3.0 IGO |
| `id-regencies.geojson` | Indonesia, kabupaten and kota (ADM2) | 519 | Badan Pusat Statistik, World Food Programme, OCHA ROAP, boundaries as of 2020, via geoBoundaries `gbOpen` release `9469f09` | CC BY 3.0 IGO |
| `world.geojson` | The world, one feature per country (242), properties `name`, `name_long`, `ISO_A2`, `ISO_A3` | 242 | Natural Earth 1:50m Admin 0 - Countries, version 5.1.1 | Public domain |
| `admin1/<ISO_A3>.geojson` | One file per country (250): its states or provinces, properties `name`, `iso_3166_2`; loaded only when a chart picks that country. Indonesia (`IDN`) has none: `id-provinces.geojson` is its map | 4,596 less Indonesia's | Natural Earth 1:10m Admin 1 - States, Provinces, version 5.1.1 | Public domain |

## Natural Earth files (BI-16 C2a)

Source: https://www.naturalearthdata.com/ (downloaded from
https://naciscdn.org/naturalearth/50m/cultural/ne_50m_admin_0_countries.zip
and
https://naciscdn.org/naturalearth/10m/cultural/ne_10m_admin_1_states_provinces.zip,
the `VERSION.txt` of both archives says 5.1.1). Natural Earth is in the
public domain: no permission or credit is needed, and none is a condition of
using the files. The map footer names it as a courtesy.

The files are produced by `ops/geo/build_natural_earth.py` (read its module
docstring for the command line); nothing is downloaded at build or test time.
Size after simplification, measured 2026-10-23: `world.geojson` 743 KB,
`admin1/` 3.8 MB in 250 files (largest `RUS` 201 KB), 5.7 MB for all of
`public/geo` including the Indonesian files.

Simplification: Douglas-Peucker per ring with a tolerance of 0.15 % of the
country's bounding-box diagonal, never below 0.004° or above 0.05°;
coordinates rounded to 0.001°; islands smaller than twice the tolerance
dropped unless that would leave a region with no shape.

Limits specific to these files:

- **De facto boundaries.** Natural Earth draws the boundaries that hold on the
  ground, which is not every government's view. The maps are for thematic
  display, not for deciding where a border runs.
- **Codes.** Somaliland, Northern Cyprus and the Siachen Glacier have no ISO
  code in Natural Earth; their `ISO_A2` and `ISO_A3` are empty, and they match
  by name only. Kosovo carries Natural Earth's own `KOS`, not an ISO code.
- **First-level regions only**, in the country's Natural Earth spelling
  (`São Paulo`); the console ignores accents and case when it joins names, and
  also accepts the ISO 3166-2 code (`BR-SP`).

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
