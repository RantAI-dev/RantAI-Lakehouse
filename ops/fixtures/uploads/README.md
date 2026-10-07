# Upload fixtures

Files an upload can contain, each with the result of reading it beside it.
Every value is invented; nothing here comes from a customer.

Two readers of one dialect (ADR 0014, decision 3) are tested against these
same files, so they cannot drift apart unnoticed:

- **The preview, in Rust:** `rust/crates/lakehouse-api/src/upload_parse.rs`.
  Its tests `every_fixture_parses_to_its_expected_json`,
  `a_cut_head_shows_a_prefix_of_what_the_whole_file_shows` and
  `the_fixture_directory_holds_exactly_the_listed_fixtures` read this
  directory.
- **The load, in Python:** `dagster/dispar_orchestrate/file_ingest.py`. Its
  tests read the same files (T7 of
  `docs/superpowers/plans/2026-10-02-upload-file.md`).

## Layout

`<name>.<ext>` is the file, byte for byte. `<name>.expected.json` is what
reading it must give:

```json
{
  "encoding": "utf-16",
  "delimiter": "\t",
  "headerRow": 4,
  "columns": ["Plnt", "Material"],
  "rows": [["8250", "0000100"]]
}
```

- `encoding`, `delimiter` and `headerRow` (a zero-based index of a RECORD, so
  blank records and cells that span lines are counted as the reader sees
  them) are what the preview must detect, and what the load is told. Given
  them, a reader must produce `columns` and `rows`.
- `columns` are the header record's cells, raw. Turning them into SQL-safe
  names is the load's job, not recorded here.
- `rows` are the records below the header that are not blank, as parsed: not
  padded, not cut, not trimmed.

## The dialect

The full statement is the module doc of `upload_parse.rs`. In short, it is
Python's `csv.reader(io.StringIO(text, newline=""), delimiter=d)` on text
decoded with `utf-8-sig` (or `utf-16`, little endian when there is no byte
order mark), with `errors="replace"`:

- `"` quotes a cell only at its start; `""` is a literal quote; the delimiter
  and line breaks inside quotes are kept byte for byte.
- A record ends at `\r\n`, `\n` or a lone `\r`. An empty line is an empty
  record.
- Cells are not trimmed. A record is blank when every cell is empty or
  whitespace (`str.isspace()`).
- Nothing is an error: text after a closing quote stays in the cell, and a
  quote that is never closed takes the rest of the file.

Settings that look harmless and are not, each pinned by a fixture:

- `newline=""`. With the default, a lone `\r` is a `csv.Error`
  (`cr_only.csv`); with `newline=None`, a `\r\n` inside a quoted cell becomes
  `\n` (`crlf_utf8_bom.csv`).
- `utf-8-sig`, not `utf-8`: a byte order mark in front of a quoted first cell
  turns its quotes into literal characters (`crlf_utf8_bom.csv`).

One difference the fixtures do not show: Python refuses a cell longer than
131,072 characters (`csv.field_size_limit()`), the Rust reader has no limit.
The load has to raise Python's, or a file the preview showed fails to load.

## Adding or changing a fixture

1. Write the file and its `.expected.json` by hand, from what the file
   means. Do not generate the expectation by running a reader on it.
2. Add `fixture!("<name>", "<ext>")` to `FIXTURES` in `upload_parse.rs`. The
   test above fails on a file the list does not name.
3. Check the expectation against Python as written above.

Do not open these files in an editor that rewrites line endings, a byte order
mark or encodings. `.gitattributes` here keeps git from converting line
endings in them too.

`sap_report_utf16.xls` is UTF-16 text with an `.xls` name, as the export that
motivated the feature was. Regenerate it with:

```python
text = ("Stock overview\r\n\r\nGenerated 24.09.2025\r\n\r\n"
        "Plnt\tMaterial\tDescription\tQty\r\n"
        "8250\t0000100\tADHESIVE\t10\r\n8250\t0000200\tPP BAND\t4\r\n")
open("sap_report_utf16.xls", "wb").write(b"\xff\xfe" + text.encode("utf-16-le"))
```
