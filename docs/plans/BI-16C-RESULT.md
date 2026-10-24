# BI-16 part C3 result — measured, the region lookup on a real engine

BI-16 part C3 (`docs/superpowers/plans/2026-10-24-bi-16c-advanced-charts-maps.md`,
decisions D7, D8) adds `RegionOf` (a point in a region of an uploaded map) on a
`ClickHouse` polygon dictionary and `Distance` on `geoDistance`. The planner's
review of C3 (S1, S2, S7) asked for the dictionary to be proved on a real,
passworded engine. Every number below comes from a command run for this
measurement on a throwaway container that was stopped at the end.

## Environment

- `ClickHouse` server 26.8.9.10 (official build), image
  `clickhouse/clickhouse-server:26.8.9.10`, the version the dev stack runs
  (the local `26.8` tag reports the same).
- A throwaway container, never the shared dev engine:
  `docker run -d --rm --name qa-ch-geo -e CLICKHOUSE_PASSWORD=<random> -p 127.0.0.1:18123:8123 clickhouse/clickhouse-server:26.8.9.10`
  (the `default` user has a password, as on any compose stack with
  `CH_PASSWORD` set). A second container without a password, and a third whose
  password contains a quote and a backslash, covered the other two cases.
- Data: the 34 provinces of `public/geo/id-provinces.geojson` (the file the
  console bundles), inserted as `JSONEachRow` into
  `console.geo_region (map_id String, name String, polygon Array(Array(Array(Tuple(Float64, Float64)))))`,
  the shape `lakehouse_bi::geo` writes. A two-square map was used first for the
  failing cases.

## S1: how a local `CLICKHOUSE` source authenticates

The code before the fix created the dictionary with
`SOURCE(CLICKHOUSE(QUERY '...'))` and no `USER`/`PASSWORD`.

| Case | Command | Result |
| --- | --- | --- |
| Password on `default`, no credentials in the DDL | `CREATE DICTIONARY ...` then `SYSTEM RELOAD DICTIONARY` | `CREATE` succeeds; the reload fails: `Code: 516 ... default: Authentication failed: password is incorrect, or there is no user with such name`. The dictionary can never load, so `RegionOf` is dead (fails closed). |
| Password on `default`, `USER 'default' PASSWORD '<pw>'` in the DDL | same | loads: `status LOADED`, `element_count 34`, `bytes_allocated 17483044`, `loading_duration 0.331` s |
| No password on `default`: `USER 'default'` only, `USER 'default' PASSWORD ''`, or nothing | same | all three load and answer |
| Password `a'b\c`, written as the literal `'a\'b\\c'` | same | loads (the escaping `\'` and `\\` is the engine's) |

Decision: the DDL carries the API's own engine login (`CH_USER`/`CH_PASSWORD`,
already in the API's configuration) as escaped literals; no new setting, no
compose change. A dedicated read-only user for `console.geo_region` would be
tighter, but it needs the engine to create that user (the API's user has no
`CREATE USER` right here: `Code: 497 ... necessary to have the grant CREATE USER`)
and a compose change, which is the planner's call; this is recorded as the
alternative.

### Is the password hidden?

Measured with the password `<pw>` in the `CREATE DICTIONARY` text:

| Where | Result |
| --- | --- |
| `SHOW CREATE DICTIONARY` | `PASSWORD '[HIDDEN]'`; 0 matches for the password |
| `system.tables.create_table_query` | `PASSWORD '[HIDDEN]'` |
| `system.dictionaries.source` | `ClickHouse: console.` (no credentials) |
| `system.query_log` (after `SYSTEM FLUSH LOGS`) | 0 rows containing the password; the logged `CREATE DICTIONARY` text carries `PASSWORD '[HIDDEN]'` |
| server log (`clickhouse-server.log`) | the password appears only in the text of the audit query that searched for it, never from the `CREATE` |
| **the engine's metadata file** `/var/lib/clickhouse/metadata/console/geo_dict_<id>.sql` | **the password in clear text** |

So the password is hidden from every SQL view and log, but sits in clear text on
the engine's own volume, where the engine's `users.d` already holds the same
secret. Consequences, written on the feature page: a password rotation leaves a
loaded dictionary working until the engine restarts, then the lookup fails
closed (upload the map again); the metadata file goes with the dictionary when
its map is deleted.

## S2: the key type

| Dictionary key | Table column | Result |
| --- | --- | --- |
| `Array(Array(Array(Array(Float64))))` (the engine documentation, the old code) | `Array(Array(Array(Tuple(Float64, Float64))))` | `SYSTEM RELOAD DICTIONARY` fails: `Code: 53 ... CAST AS Array can only be performed between same-dimensional array types` (TYPE_MISMATCH) |
| `Array(Array(Array(Tuple(Float64, Float64))))` | same | loads |

The key is now the table's own type.

## `RegionOf` on three points (the compiled form)

`dictGetOrDefault('console.geo_dict_<id>', 'name', (toFloat64(lon), toFloat64(lat)), '')`
on the provinces dictionary (order of the tuple: longitude, latitude):

| Place | lat, lon | Region |
| --- | --- | --- |
| Jakarta | -6.2088, 106.8456 | `Dki Jakarta` |
| Bandung | -6.9175, 107.6191 | `Jawa Barat` |
| Denpasar | -8.65, 115.2167 | `Bali` |
| open sea | -30.0, 90.0 | empty string (the default) |

The same checks run as the ignored test
`formula::compile::tests::live_region_of_names_three_provinces_and_distance_matches_a_published_figure`
(see below).

## `Distance` (S7)

- `geoDistance(200, 100, 0, 0)` returns `8963269` and does not throw
  (planner's measurement, reproduced here): the engine accepts coordinates
  outside the range, so the compiled form is
  `if(abs(lat1) <= 90 AND abs(lat2) <= 90 AND abs(lon1) <= 180 AND abs(lon2) <= 180, geoDistance(...), NULL)`,
  and a literal outside the range is refused at compile time with a sentence.
- Published figure: Soekarno-Hatta airport (CGK, -6.1256, 106.6559) to Husein
  Sastranegara airport (BDO, -6.9006, 107.5763) is 133 km by Vincenty's formula
  (airmilescalculator.com, `distance/bdo-to-cgk`). `geoDistance` returned
  `133.0729813407716` km, a difference of 0.05 %, inside the 1 % the plan asks.
  (City-centre points: Jakarta to Bandung, `geoDistance(106.8456, -6.2088, 107.6191, -6.9175)`
  = 116,016.69 m, which agrees with the roughly 115 km a haversine calculator
  quotes for city centres.)

## The ignored live test

```
LIVE_CH_URL=http://127.0.0.1:18123 LIVE_CH_USER=default LIVE_CH_PASSWORD=<pw> \
  cargo test -p lakehouse-bi --lib live_region_of -- --ignored
```

It loads the provinces, creates the dictionary with `dictionary_ddl` as the
API does, checks the three provinces and the open sea, checks `Distance` against
133 km within 1 %, checks that an out-of-range input gives NULL, and drops what
it made. Its result on the throwaway engine is in the plan's handoff entry
"Review fixes S1–S8 (C3)".

## Not measured

- The memory and load time of a 20,000-region dictionary (34 provinces took
  17.5 MB and 0.33 s; a linear estimate is not a measurement).
- `mutations_sync` on `ALTER TABLE console.geo_region DELETE` under load.
- Whether the API's engine user has `CREATE DICTIONARY` on a locked-down stack:
  the dev user is the engine's `default`, which has every right; a stack with a
  restricted API user would show a refusal at upload time, which leaves the
  lookup off and the map drawing (the lifecycle's fail-closed path, covered by
  the route test with a mock engine).
