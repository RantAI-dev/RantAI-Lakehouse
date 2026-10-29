# Hostile upload fixtures (`SEC-22`)

Files that ask a reader for more than the machine has, so that the API's
memory bound is a fact and not a hope. Rebuild them with:

```bash
python3 ops/fixtures/uploads/hostile/make_hostile.py
```

The script uses only the standard library and rebuilds both files byte for
byte.

## `huge_shared_strings.xlsx`

A real workbook (`../../workbooks/stock.xlsx`) with a shared-strings part
whose header declares **two billion** strings and holds three.

`calamine` 0.37 reads that header before it reads a single string and
reserves room for all of them (`src/xlsx/mod.rs`, `read_shared_strings`:
`self.strings.reserve(n)`), so the file asks for 48 GB — 2,000,000,000
strings of 24 bytes — before any cell is looked at. The allocation fails and
Rust aborts the process. `catch_unwind` cannot catch that: it catches a
panic, and an allocation failure is not one.

Measured on 2026-10-28 (`SEC-22` T1, `rust/crates/lakehouse-api/tests/
hostile_uploads.rs`), under `ulimit -v 524288`:

```text
[sec22] huge_shared_strings.xlsx: child status ExitStatus(unix_wait_status(134))
[sec22] huge_shared_strings.xlsx: memory allocation of 48000000000 bytes failed
```

134 is 128 + SIGABRT. The process that read it was the child, not the API.

## `huge_footer.parquet`

A Parquet file of 123 bytes whose footer declares a row group and a column
chunk of **one tebibyte**, and whose schema says one row of one string
column.

The reader checks the footer against the file before it trusts it
(`lakehouse_api::upload_parquet`, `check_layout`: a row group larger than
the data region is refused), so this file is refused with the reader's own
sentence and costs nothing:

```text
[sec22] huge_footer.parquet: child status ExitStatus(unix_wait_status(0))
[sec22] huge_footer.parquet: REFUSED: This file could not be opened as a Parquet file.
```

It is here as the counter-example: the Parquet reader bounds what the footer
declares, and the workbook reader does not.

## `huge_list_footer.parquet`

A Parquet file of 48 bytes whose footer declares **two billion row groups**
and holds none.

The list header of the Thrift compact protocol carries the count BEFORE the
elements, and the reader reserves room for the whole list before it reads the
first one (`parquet` 58.4.0, `file/metadata/thrift/mod.rs:804`:
`Vec::with_capacity(list_ident.size)`). The reservation is 192 GB, so the
allocation fails and the process that read it aborts.

This is the case the Parquet reader does NOT bound: the count is in the
footer's own list header, and there is nothing to check it against but the
file — which is why the footer is read by the CHILD (`SEC-22` P1,
`PR #187` review BLOCKER 1) and never by the API:

```text
$ lakehouse-api inspect-upload inspect-parquet 48 < huge_list_footer.parquet
memory allocation of 192000000000 bytes failed
Aborted (exit 134)
```

The `.xls` case needs no fixture

`SEC-22`'s plan asks for an `.xls` whose shared string table declares a huge
count, *if the reader reads that count*. `calamine` 0.37 does not:
`src/xls.rs`, `parse_sst`, skips the `cstTotal` and `cstUnique` headers and
reads the strings to the end of the record, so the count is never turned
into a reservation. A count that lies costs a lying count nothing, and there
is no `.xls` fixture to commit.
