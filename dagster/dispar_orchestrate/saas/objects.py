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
    (merge key); empty only for an object re-read in full whose records carry no identifier
    (a Workday report). `deletes`: whether the vendor reports deleted records in the
    same read (decision 7). `full_reason`: required when `cursor` is `None`.
    """

    name: str
    primary_key: tuple[str, ...]
    cursor: str | None = None
    deletes: bool = False
    full_reason: str | None = None
    lag_seconds: int = 0
    # PR #216 final review BLOCKER 17: a vendor that continues from a delta link (Graph's
    # `@odata.deltaLink`, Dataverse change tracking) replays the link and sends every row that
    # changed since it was issued. Some of those rows carry a cursor value BELOW the last run's
    # highest: an edit made while run 1 was paging (stamped before the item it edited was read),
    # or a change stamped before a delete's `now()` mark. Filtering them by the cursor loses them
    # silently, so a delta object is read without dlt's incremental filter and merged on the key.
    delta: bool = False
    # A name for a person where `name` is an id (SharePoint: `Site / List`, slice 5 review BLOCKER 14). Shown by
    # discovery only; the ingest spec keys the object by `name`.
    label: str | None = None

    def __post_init__(self) -> None:
        if not self.name.strip():
            raise ValueError("an object needs a name")
        # Slice 6 (Workday): a custom report has no identifier, so a report that is re-read in full
        # may declare no key (the table is replaced on every run, nothing is merged). An object that
        # continues from a cursor merges on its key and still needs one.
        if not self.primary_key and self.cursor is not None:
            raise ValueError(f"object {self.name!r} needs a primary key")
        if self.cursor is None and not (self.full_reason or "").strip():
            raise ValueError(f"object {self.name!r} has no cursor and gives no reason it is re-read in full")
        if self.lag_seconds < 0 or (self.lag_seconds and self.cursor is None):
            raise ValueError(f"object {self.name!r} has a lag but no cursor, or a negative one")
        if self.cursor is not None and self.full_reason:
            raise ValueError(f"object {self.name!r} has a cursor and also a reason it is re-read in full")
        # BLOCKER 17: a delta object is continued from its link, so it must name the cursor the
        # console offers it incremental loads by, and the key the sink merges it on.
        if self.delta and (self.cursor is None or not self.primary_key):
            raise ValueError(f"object {self.name!r} continues from a delta link, so it needs a cursor and a primary key")

    @property
    def incremental(self) -> bool:
        return self.cursor is not None

    def load_description(self) -> str:
        """The words for the connector page (decision 6)."""
        return "incremental" if self.incremental else f"re-read in full: {self.full_reason}"
