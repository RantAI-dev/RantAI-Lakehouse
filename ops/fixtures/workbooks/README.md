# Workbook fixtures

Small Excel workbooks for `rust/crates/lakehouse-api/src/upload_workbook.rs`
and the upload routes. Every value is invented.

| File | What it is |
| --- | --- |
| `stock.xlsx`, `stock.xls` | The same five sheets: `Stock`, `Quirks` (one cell per conversion rule), `Offset` (data starting at C3, an empty row inside), `Hidden notes` (hidden), `Empty` (no cells). |
| `formula.xlsx` | Formula cells with stored results (a number, a string, an error). |
| `date1904.xlsx` | Dates in the 1904 date system. |
| `plain.zip` | A zip that is not a workbook. |

`make_workbooks.py` writes all of them (`openpyxl` for `.xlsx`, `xlwt` for
`.xls`; neither is a repo dependency). The bytes are not reproducible, and
nothing compares them: the tests compare what the files hold, against the
literals in `upload_workbook.rs` and against
`../uploads/converted_sheet.csv`.
