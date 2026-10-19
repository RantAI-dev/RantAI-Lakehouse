"""What a vendor module declares per object (`SRC-3` AD6; decisions 6 and 7).

The Rust side saves an `incrementalKey` only for an object the console offers
it for, and the console offers it from `ObjectDecl.cursor`. An object with no
cursor is re-read in full on every run, and `full_reason` says why in words a
person reads on the connector page ("incremental" or "re-read in full:
<reason>"). A vendor module that declares neither a cursor nor a reason is a
bug in the module, refused when the declaration is built.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ObjectDecl:
    """One object of a vendor.

    `cursor`: the field whose value the vendor's API can filter by ("last
    modified"), or `None`. `primary_key`: the fields that identify one record
    (merge key). `deletes`: whether the vendor reports deleted records in the
    same read (decision 7). `full_reason`: required when `cursor` is `None`.
    """

    name: str
    primary_key: tuple[str, ...]
    cursor: str | None = None
    deletes: bool = False
    full_reason: str | None = None

    def __post_init__(self) -> None:
        if not self.name.strip():
            raise ValueError("an object needs a name")
        if not self.primary_key:
            raise ValueError(f"object {self.name!r} needs a primary key")
        if self.cursor is None and not (self.full_reason or "").strip():
            raise ValueError(f"object {self.name!r} has no cursor and gives no reason it is re-read in full")
        if self.cursor is not None and self.full_reason:
            raise ValueError(f"object {self.name!r} has a cursor and also a reason it is re-read in full")

    @property
    def incremental(self) -> bool:
        return self.cursor is not None

    def load_description(self) -> str:
        """The words for the connector page (decision 6)."""
        return "incremental" if self.incremental else f"re-read in full: {self.full_reason}"
