"""Rebuilds the hostile upload fixtures of this directory.

    python3 ops/fixtures/uploads/hostile/make_hostile.py

No dependency beyond the standard library, and no network: the two files are
built here, byte for byte, every time.

Written here:

- `huge_shared_strings.xlsx`: a real workbook (`../workbooks/stock.xlsx`) with a
  shared-strings part whose header declares two billion strings. A reader that
  believes the header reserves room for all of them before it reads one, and
  the reservation is what ends the process (SEC-22).
- `huge_footer.parquet`: a Parquet file whose footer declares a row group and
  a column chunk of one tebibyte, in a file of a few hundred bytes. A reader
  that trusts the footer reads past the end of the file.

The `.xls` case of the SEC-22 plan needs no fixture: `calamine` 0.37 does not
read the shared string table's count at all (`src/xls.rs`, `parse_sst`: it
skips the `cstTotal` and `cstUnique` headers and reads the strings to the end
of the record), so a count that lies costs a lying count nothing. The
`SEC-22` handoff quotes that.
"""

from __future__ import annotations

import pathlib
import zipfile

HERE = pathlib.Path(__file__).parent
STOCK = HERE.parent.parent / "workbooks" / "stock.xlsx"

# The count the shared-strings header declares. `calamine` reserves
# `count * size_of::<String>()` (24 bytes on this platform) for it before it
# reads a single string, so this asks for 48 GB — far past any machine, and
# past the limit the API's child process sets for itself.
DECLARED_STRINGS = 2_000_000_000

# The size the Parquet footer declares for a row group and for its column
# chunk: one tebibyte, in a file of a few hundred bytes.
DECLARED_BYTES = 1 << 40

SHARED_STRINGS_PART = "xl/sharedStrings.xml"
SHARED_STRINGS_TYPE = (
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"
)
SHARED_STRINGS_REL = (
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings"
)


def hostile_shared_strings() -> bytes:
    """A shared-strings part whose header lies about how many it holds."""
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        f'count="{DECLARED_STRINGS}" uniqueCount="{DECLARED_STRINGS}">'
        "<si><t>sku</t></si><si><t>name</t></si><si><t>qty</t></si>"
        "</sst>"
    ).encode("utf-8")


def build_xlsx() -> None:
    """The real workbook plus a shared-strings part that lies about its size."""
    source = zipfile.ZipFile(STOCK)
    target = HERE / "huge_shared_strings.xlsx"
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as out:
        for entry in source.infolist():
            data = source.read(entry.filename)
            if entry.filename == "[Content_Types].xml":
                data = data.replace(
                    b"</Types>",
                    f'<Override PartName="/{SHARED_STRINGS_PART}" '
                    f'ContentType="{SHARED_STRINGS_TYPE}" /></Types>'.encode(),
                )
            elif entry.filename == "xl/_rels/workbook.xml.rels":
                data = data.replace(
                    b"</Relationships>",
                    b'<Relationship Type="'
                    + SHARED_STRINGS_REL.encode()
                    + b'" Target="sharedStrings.xml" Id="rId100" /></Relationships>',
                )
            out.writestr(entry, data)
        out.writestr(SHARED_STRINGS_PART, hostile_shared_strings())
    source.close()
    print(f"wrote {target.name}: {target.stat().st_size} bytes")


# ── Thrift compact protocol, the one Parquet's footer is written in ────────

STOP = 0x00
BOOLEAN_TRUE = 0x01
I32 = 0x05
I64 = 0x06
BINARY = 0x08
LIST = 0x09
STRUCT = 0x0C


def varint(value: int) -> bytes:
    """An unsigned LEB128 varint."""
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def zigzag(value: int) -> int:
    """The compact protocol's signed encoding: `(n << 1) ^ (n >> 63)`."""
    return (value << 1) ^ (value >> 63)


def field_header(field_id: int, last: int, field_type: int) -> tuple[bytes, int]:
    """The header of one field: its type and its id, as a delta when it fits."""
    delta = field_id - last
    if 0 < delta <= 15:
        return bytes([(delta << 4) | field_type]), field_id
    return bytes([0xF0 | field_type]) + varint(zigzag(field_id)), field_id


def write_value(out: bytearray, field_type: int, value: object) -> None:
    """The bytes of one field's value."""
    if field_type in (I32, I64):
        out += varint(zigzag(int(value)))  # type: ignore[arg-type]
    elif field_type == BINARY:
        raw = value if isinstance(value, bytes) else str(value).encode("utf-8")
        out += varint(len(raw))
        out += raw
    elif field_type == LIST:
        element_type, elements = value  # type: ignore[misc]
        if len(elements) < 15:
            out.append((len(elements) << 4) | element_type)
        else:
            out.append(0xF0 | element_type)
            out += varint(len(elements))
        for element in elements:
            write_value(out, element_type, element)
    elif field_type == STRUCT:
        out += write_struct(value)  # type: ignore[arg-type]
    else:
        raise ValueError(f"unsupported type {field_type}")


def write_struct(fields: list[tuple[int, int, object]]) -> bytes:
    """One struct: its fields in id order, then the stop marker."""
    out = bytearray()
    last = 0
    for field_id, field_type, value in sorted(fields, key=lambda f: f[0]):
        header, last = field_header(field_id, last, field_type)
        out += header
        write_value(out, field_type, value)
    out.append(STOP)
    return bytes(out)


def parquet_footer() -> bytes:
    """A Parquet `FileMetaData` that declares far more than the file holds."""
    # SchemaElement: name (4), num_children (5), type (1), repetition_type (3),
    # converted_type (6).
    root = [(4, BINARY, "schema"), (5, I32, 1)]
    leaf = [
        (1, I32, 6),  # BYTE_ARRAY
        (3, I32, 1),  # OPTIONAL
        (4, BINARY, "id"),
        (6, I32, 0),  # UTF8
    ]
    # ColumnMetaData: type (1), encodings (2), path_in_schema (3), codec (4),
    # num_values (5), total_uncompressed_size (6), total_compressed_size (7),
    # data_page_offset (9).
    meta_data = [
        (1, I32, 6),
        (2, LIST, (I32, [0])),
        (3, LIST, (BINARY, [b"id"])),
        (4, I32, 0),
        (5, I64, 1),
        (6, I64, DECLARED_BYTES),
        (7, I64, DECLARED_BYTES),
        (9, I64, 4),
    ]
    # ColumnChunk: file_offset (2), meta_data (3).
    column = [(2, I64, 4), (3, STRUCT, meta_data)]
    # RowGroup: columns (1), total_byte_size (2), num_rows (3), file_offset (5),
    # total_compressed_size (6).
    row_group = [
        (1, LIST, (STRUCT, [column])),
        (2, I64, DECLARED_BYTES),
        (3, I64, 1),
        (5, I64, 4),
        (6, I64, DECLARED_BYTES),
    ]
    # FileMetaData: version (1), schema (2), num_rows (3), row_groups (4),
    # created_by (6).
    return write_struct(
        [
            (1, I32, 1),
            (2, LIST, (STRUCT, [root, leaf])),
            (3, I64, 1),
            (4, LIST, (STRUCT, [row_group])),
            (6, BINARY, b"SEC-22 hostile fixture"),
        ]
    )


# The count the Parquet footer's row-group list declares. The list header of
# the compact protocol carries the count BEFORE the elements, and the reader
# reserves room for the whole list before it reads the first one
# (`parquet::file::metadata::thrift`: `Vec::with_capacity(list_ident.size)`).
# So a footer of a few dozen bytes asks for hundreds of gigabytes, and the
# allocation failure ends the process that made it (SEC-22, `PR #187` review
# BLOCKER 1).
DECLARED_GROUPS = 2_000_000_000


def build_parquet() -> None:
    """A Parquet file whose footer declares a tebibyte of data it does not hold."""
    footer = parquet_footer()
    target = HERE / "huge_footer.parquet"
    with target.open("wb") as out:
        out.write(b"PAR1")
        out.write(footer)
        out.write(len(footer).to_bytes(4, "little"))
        out.write(b"PAR1")
    print(f"wrote {target.name}: {target.stat().st_size} bytes")


def huge_list_footer() -> bytes:
    """A Parquet `FileMetaData` whose row-group list declares billions of groups.

    The list header carries the count, and the elements are never there: the
    reader reserves room for the whole list before it reads the first
    element, which is the reservation that ends the process.
    """
    root = [(4, BINARY, "schema"), (5, I32, 1)]
    leaf = [
        (1, I32, 6),  # BYTE_ARRAY
        (3, I32, 1),  # OPTIONAL
        (4, BINARY, "id"),
        (6, I32, 0),  # UTF8
    ]
    out = bytearray()
    last = 0
    header, last = field_header(1, last, I32)
    out += header
    write_value(out, I32, 1)  # version
    header, last = field_header(2, last, LIST)
    out += header
    write_value(out, LIST, (STRUCT, [root, leaf]))  # schema
    header, last = field_header(3, last, I64)
    out += header
    write_value(out, I64, 1)  # num_rows
    header, last = field_header(4, last, LIST)
    out += header
    # The row groups: the count is in the header, the elements are not there.
    out.append(0xF0 | STRUCT)
    out += varint(DECLARED_GROUPS)
    out.append(STOP)
    return bytes(out)


def build_huge_list_parquet() -> None:
    """A Parquet file whose footer asks for more memory than the machine has."""
    footer = huge_list_footer()
    target = HERE / "huge_list_footer.parquet"
    with target.open("wb") as out:
        out.write(b"PAR1")
        out.write(footer)
        out.write(len(footer).to_bytes(4, "little"))
        out.write(b"PAR1")
    print(f"wrote {target.name}: {target.stat().st_size} bytes")


if __name__ == "__main__":
    build_xlsx()
    build_parquet()
    build_huge_list_parquet()
