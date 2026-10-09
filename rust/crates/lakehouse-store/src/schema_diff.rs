//! The comparison of a source table's columns with the last accepted ones,
//! and the policy that says what to do about the difference (`SRC-8`
//! tasks 4, decisions D1-D4). Pure: no I/O, no clock, no database.
//!
//! # Why it sits in `lakehouse-store`
//!
//! It works on [`crate::schema_change`]'s own types ([`ObservedColumn`],
//! [`ChangeKind`], [`ChangeStatus`], [`SchemaChangePolicy`]) and turns its
//! output into [`NewChange`] rows; the API crate only wires the two
//! together. Keeping it free of `PgPool` makes every row of the spec's
//! Target table a plain unit test.
//!
//! # The two functions
//!
//! [`diff`] finds what differs. A rename is a removal plus an addition (it
//! cannot be told from them); names are compared exactly, case-sensitively,
//! because a source that has both `Id` and `id` has two columns, and the
//! Bronze table keeps the spelling it was given.
//!
//! [`decide`] applies the policy. Breaking changes (a removed column, a
//! narrowed or unclassified type, a changed primary key) make the table
//! WAIT under every policy, as long as the source can hold the table back
//! (`can_hold_back`). The property the tests state in words: a breaking
//! change never answers "load the full column list" while the source can
//! hold back.
//!
//! # Type classification (D3)
//!
//! One table of widenings, in [`classify_type`]: a larger integer, a float
//! to a double, a longer text or text to unbounded text (text to text
//! only). The reverse of each is narrowed. Everything else that differs is
//! [`TypeChange::Other`] and counts as breaking: an unknown change is
//! never assumed safe (fail closed). `SRC-8 review BLOCKER 1`: a non-text
//! type becoming text and any change to a decimal's precision or scale are
//! `Other`, because the load of such a change fails for a SQL source
//! (`docs/plans/SRC-8-RESULT.md`).

use crate::schema_change::{
    AcceptedSchema, ChangeKind, ChangeStatus, NewChange, ObservedColumn, SchemaChangePolicy,
};

/// A table's columns and primary key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableShape {
    /// Columns in the source's order.
    pub columns: Vec<ObservedColumn>,
    /// Primary key column names.
    pub primary_key: Vec<String>,
}

impl From<&AcceptedSchema> for TableShape {
    fn from(accepted: &AcceptedSchema) -> Self {
        Self {
            columns: accepted.columns.clone(),
            primary_key: accepted.primary_key.clone(),
        }
    }
}

/// How a column's type changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeChange {
    /// The new type holds everything the old one held (non-breaking).
    Widened,
    /// The reverse of a widening (breaking).
    Narrowed,
    /// Neither list covers it (breaking, D3).
    Other,
}

/// One difference between the accepted and the observed shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A column the accepted shape did not have.
    ColumnAdded {
        /// The column.
        name: String,
        /// Its type at the source.
        type_name: String,
    },
    /// A column the accepted shape had and the source no longer shows.
    ColumnRemoved {
        /// The column.
        name: String,
        /// Its last accepted type.
        type_name: String,
    },
    /// The same column with another type.
    TypeChanged {
        /// The column.
        name: String,
        /// The accepted type.
        before: String,
        /// The observed type.
        after: String,
        /// Widened, narrowed or other.
        change: TypeChange,
    },
    /// The primary key's columns differ (compared as a set).
    PrimaryKeyChanged {
        /// The accepted key.
        before: Vec<String>,
        /// The observed key.
        after: Vec<String>,
    },
}

impl Change {
    /// Whether the change makes the table wait under every policy.
    #[must_use]
    pub fn is_breaking(&self) -> bool {
        match self {
            Self::ColumnAdded { .. } => false,
            Self::ColumnRemoved { .. } | Self::PrimaryKeyChanged { .. } => true,
            Self::TypeChanged { change, .. } => *change != TypeChange::Widened,
        }
    }

    /// The stored kind.
    #[must_use]
    pub fn kind(&self) -> ChangeKind {
        match self {
            Self::ColumnAdded { .. } => ChangeKind::ColumnAdded,
            Self::ColumnRemoved { .. } => ChangeKind::ColumnRemoved,
            Self::TypeChanged { .. } => ChangeKind::TypeChanged,
            Self::PrimaryKeyChanged { .. } => ChangeKind::PrimaryKeyChanged,
        }
    }
}

/// What to do with the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Load it (with [`Decision::columns`] when that is `Some`).
    Load,
    /// Do not load it; a person decides.
    Wait,
}

/// A change with its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedChange {
    /// The change.
    pub change: Change,
    /// `Applied` or `Pending`.
    pub status: ChangeStatus,
}

impl ClassifiedChange {
    /// The row to store. Values are column and type names only.
    #[must_use]
    pub fn to_new_change(&self) -> NewChange {
        let (column_name, before_value, after_value) = match &self.change {
            Change::ColumnAdded { name, type_name } => {
                (name.clone(), None, Some(type_name.clone()))
            }
            Change::ColumnRemoved { name, type_name } => {
                (name.clone(), Some(type_name.clone()), None)
            }
            Change::TypeChanged {
                name,
                before,
                after,
                ..
            } => (name.clone(), Some(before.clone()), Some(after.clone())),
            Change::PrimaryKeyChanged { before, after } => (
                String::new(),
                Some(before.join(", ")),
                Some(after.join(", ")),
            ),
        };
        NewChange {
            kind: self.change.kind(),
            column_name,
            before_value,
            after_value,
            breaking: self.change.is_breaking(),
            status: self.status,
        }
    }
}

/// The answer for one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Load or wait.
    pub action: Action,
    /// With `Load`: the columns to load, or `None` for all of them.
    pub columns: Option<Vec<String>>,
    /// Whether the connector as a whole is paused (policy `pause`), except
    /// when a change waits that Approve refuses (`SRC-8 review BLOCKER 3a`).
    pub pause_connector: bool,
    /// The changes with their status.
    pub changes: Vec<ClassifiedChange>,
}

/// Compare the observed shape with the last accepted one. `previous: None`
/// is the first observation: the baseline, no changes.
#[must_use]
pub fn diff(previous: Option<&TableShape>, observed: &TableShape) -> Vec<Change> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    // Exact, case-sensitive name comparison (see the module doc).
    for column in &observed.columns {
        if !previous.columns.iter().any(|p| p.name == column.name) {
            changes.push(Change::ColumnAdded {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
            });
        }
    }
    for column in &previous.columns {
        if !observed.columns.iter().any(|o| o.name == column.name) {
            changes.push(Change::ColumnRemoved {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
            });
        }
    }
    for column in &observed.columns {
        let Some(old) = previous.columns.iter().find(|p| p.name == column.name) else {
            continue;
        };
        if let Some(change) = classify_type(&old.type_name, &column.type_name) {
            changes.push(Change::TypeChanged {
                name: column.name.clone(),
                before: old.type_name.clone(),
                after: column.type_name.clone(),
                change,
            });
        }
    }
    // A key is a set of columns: the same columns in another order enforce
    // the same uniqueness, so only a different set is a change.
    let mut before = previous.primary_key.clone();
    let mut after = observed.primary_key.clone();
    before.sort();
    after.sort();
    if before != after {
        changes.push(Change::PrimaryKeyChanged {
            before: previous.primary_key.clone(),
            after: observed.primary_key.clone(),
        });
    }
    changes
}

/// Apply the policy to the changes of one table.
///
/// `can_hold_back` is `false` for the after-load phase of files, REST,
/// `MongoDB`, Kafka and SFTP (decision D4): the changes are already in the
/// table, so they are recorded as applied and the answer is always `Load`.
/// `pause` still sets `pause_connector` there, so LATER runs stop.
///
/// When a breaking change and a non-breaking one arrive together, the whole
/// table waits and every change is pending: approving the table accepts
/// the observed shape as a whole, so the non-breaking part cannot be
/// applied on its own.
#[must_use]
pub fn decide(
    policy: SchemaChangePolicy,
    can_hold_back: bool,
    previous: Option<&TableShape>,
    observed: &TableShape,
    changes: &[Change],
) -> Decision {
    let classify = |status: ChangeStatus| -> Vec<ClassifiedChange> {
        changes
            .iter()
            .cloned()
            .map(|change| ClassifiedChange { change, status })
            .collect()
    };
    if changes.is_empty() {
        return Decision {
            action: Action::Load,
            columns: None,
            pause_connector: false,
            changes: Vec::new(),
        };
    }
    let pause_connector = policy == SchemaChangePolicy::Pause;
    if !can_hold_back {
        return Decision {
            action: Action::Load,
            columns: None,
            pause_connector,
            changes: classify(ChangeStatus::Applied),
        };
    }
    if changes.iter().any(Change::is_breaking) {
        // `SRC-8 review BLOCKER 3a`: a type change the existing column
        // cannot hold is refused by Approve, so only the source putting the
        // column back clears it, and that is seen only by a connector that
        // keeps running. Pausing it would leave it stuck for good: the table
        // waits, the rest of the connector runs.
        let cannot_be_approved = changes.iter().any(|c| {
            matches!(
                c,
                Change::TypeChanged {
                    change: TypeChange::Other,
                    ..
                }
            )
        });
        return Decision {
            action: Action::Wait,
            columns: None,
            pause_connector: pause_connector && !cannot_be_approved,
            changes: classify(ChangeStatus::Pending),
        };
    }
    match policy {
        SchemaChangePolicy::ApplyNonBreaking | SchemaChangePolicy::ApplyAll => Decision {
            action: Action::Load,
            columns: None,
            pause_connector: false,
            changes: classify(ChangeStatus::Applied),
        },
        SchemaChangePolicy::AskFirst => Decision {
            action: Action::Load,
            // The previously accepted columns that still exist, in the
            // source's current order. A widened column is among them: it
            // keeps loading, and where the old column cannot hold the new
            // values they land in a second column (D9).
            columns: Some(
                observed
                    .columns
                    .iter()
                    .filter(|c| {
                        previous.is_some_and(|p| p.columns.iter().any(|o| o.name == c.name))
                    })
                    .map(|c| c.name.clone())
                    .collect(),
            ),
            pause_connector: false,
            changes: classify(ChangeStatus::Pending),
        },
        SchemaChangePolicy::Pause => Decision {
            action: Action::Wait,
            columns: None,
            pause_connector: true,
            changes: classify(ChangeStatus::Pending),
        },
    }
}

/// [`diff`] then [`decide`].
#[must_use]
pub fn evaluate(
    policy: SchemaChangePolicy,
    can_hold_back: bool,
    previous: Option<&TableShape>,
    observed: &TableShape,
) -> Decision {
    let changes = diff(previous, observed);
    decide(policy, can_hold_back, previous, observed, &changes)
}

// ---- type classification --------------------------------------------------

/// A source type string, parsed far enough to compare.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Parsed {
    /// Integer family, by size rank (1 = 8 bit .. 5 = 64 bit). The display
    /// width of `int(11)` is ignored.
    Integer(u8),
    /// Float family: 1 = single, 2 = double.
    Float(u8),
    /// Bounded text `char`/`varchar`/`nvarchar` of this length.
    BoundedText { base: String, length: u32 },
    /// Unbounded text: `text`, `varchar` without a length, `(max)`.
    UnboundedText,
    /// `numeric`/`decimal`; `None` precision is unconstrained.
    Numeric {
        precision: Option<u32>,
        scale: Option<u32>,
    },
    /// Anything else, compared by its normalised spelling only.
    Opaque(String),
}

/// Lower-case and drop every space: `Double Precision` and `numeric(10, 2)`
/// become `doubleprecision` and `numeric(10,2)`.
fn normalise(type_name: &str) -> String {
    type_name
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_type(type_name: &str) -> Parsed {
    let normalised = normalise(type_name);
    let (base, args, suffix) = match normalised.split_once('(') {
        Some((base, rest)) => match rest.split_once(')') {
            Some((args, suffix)) => (base, args, suffix),
            None => return Parsed::Opaque(normalised),
        },
        None => (normalised.as_str(), "", ""),
    };
    // `varchar(40)[]`, `int unsigned` (after space removal `intunsigned`):
    // anything this table does not know is compared by spelling.
    if !suffix.is_empty() {
        return Parsed::Opaque(normalised);
    }
    let numbers: Vec<u32> = if args.is_empty() {
        Vec::new()
    } else {
        let parsed: Option<Vec<u32>> = args.split(',').map(|a| a.parse().ok()).collect();
        match (parsed, args) {
            (Some(numbers), _) => numbers,
            (None, "max") => return text_or_opaque(base, None, &normalised),
            (None, _) => return Parsed::Opaque(normalised),
        }
    };
    match base {
        "tinyint" | "int1" => Parsed::Integer(1),
        "smallint" | "int2" => Parsed::Integer(2),
        "mediumint" => Parsed::Integer(3),
        "int" | "integer" | "int4" => Parsed::Integer(4),
        "bigint" | "int8" => Parsed::Integer(5),
        "real" | "float4" => Parsed::Float(1),
        "doubleprecision" | "float8" | "double" => Parsed::Float(2),
        "numeric" | "decimal" => Parsed::Numeric {
            precision: numbers.first().copied(),
            // `numeric(p)` has scale 0.
            scale: numbers
                .first()
                .map(|_| numbers.get(1).copied().unwrap_or(0)),
        },
        _ => text_or_opaque(base, numbers.first().copied(), &normalised),
    }
}

/// The text family; `length: None` is "no length given" (or `max`).
fn text_or_opaque(base: &str, length: Option<u32>, normalised: &str) -> Parsed {
    match base {
        "text" | "ntext" | "longtext" | "clob" | "nclob" | "string" => Parsed::UnboundedText,
        "varchar" | "nvarchar" | "charactervarying" => match length {
            Some(length) => Parsed::BoundedText {
                base: canonical_text_base(base),
                length,
            },
            None => Parsed::UnboundedText,
        },
        // `char` without a length is `char(1)`.
        "char" | "nchar" | "character" => Parsed::BoundedText {
            base: canonical_text_base(base),
            length: length.unwrap_or(1),
        },
        _ => Parsed::Opaque(normalised.to_owned()),
    }
}

fn canonical_text_base(base: &str) -> String {
    match base {
        "charactervarying" => "varchar",
        "character" => "char",
        other => other,
    }
    .to_owned()
}

/// How `before` became `after`; `None` when the type did not change.
///
/// The ONE table of widenings (D3), read top to bottom. `SRC-8 review
/// BLOCKER 1`: `docs/plans/SRC-8-RESULT.md` measured that the load of an
/// integer column that became text FAILS for a SQL source (Iceberg refuses
/// `long -> string`) and keeps failing, so only changes the existing column
/// can hold are widenings:
///
/// | before | after | result |
/// | --- | --- | --- |
/// | integer family | a larger one (`tinyint` < `smallint` < `mediumint` < `int` < `bigint`) | widened |
/// | `real`/`float4` | `double precision`/`float8`/`double` | widened |
/// | `varchar(n)`/`char(n)`/`nvarchar(n)`/`nchar(n)` | a larger `n`, or unbounded text (text to text only) | widened |
///
/// The reverse of a row is narrowed; every other difference is other
/// (breaking), a decimal's precision or scale and any type that becomes
/// text included.
#[must_use]
pub fn classify_type(before: &str, after: &str) -> Option<TypeChange> {
    use Parsed::{BoundedText, Float, Integer, UnboundedText};
    let (old, new) = (parse_type(before), parse_type(after));
    if old == new {
        return None;
    }
    Some(match (&old, &new) {
        (Integer(a), Integer(b)) | (Float(a), Float(b)) => ordered(*a, *b)?,
        (UnboundedText, UnboundedText) => return None,
        (BoundedText { length: a, base: x }, BoundedText { length: b, base: y }) => {
            match a.cmp(b) {
                std::cmp::Ordering::Less => TypeChange::Widened,
                std::cmp::Ordering::Greater => TypeChange::Narrowed,
                // Same length, another spelling (`char(10)` to `varchar(10)`):
                // padding differs, so this is not assumed safe.
                std::cmp::Ordering::Equal if x == y => return None,
                std::cmp::Ordering::Equal => TypeChange::Other,
            }
        }
        // `SRC-8 review BLOCKER 1`: no rule for decimals; any change of
        // precision or scale falls to `Other` below.
        (BoundedText { .. }, UnboundedText) => TypeChange::Widened,
        (UnboundedText, BoundedText { .. }) => TypeChange::Narrowed,
        _ => TypeChange::Other,
    })
}

fn ordered(old: u8, new: u8) -> Option<TypeChange> {
    match old.cmp(&new) {
        std::cmp::Ordering::Less => Some(TypeChange::Widened),
        std::cmp::Ordering::Greater => Some(TypeChange::Narrowed),
        std::cmp::Ordering::Equal => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, type_name: &str) -> ObservedColumn {
        ObservedColumn {
            name: name.to_owned(),
            type_name: type_name.to_owned(),
            nullable: true,
        }
    }

    fn shape(columns: &[(&str, &str)], key: &[&str]) -> TableShape {
        TableShape {
            columns: columns.iter().map(|(n, t)| col(n, t)).collect(),
            primary_key: key.iter().map(|k| (*k).to_owned()).collect(),
        }
    }

    fn base() -> TableShape {
        shape(&[("id", "integer"), ("note", "text")], &["id"])
    }

    const POLICIES: [SchemaChangePolicy; 4] = [
        SchemaChangePolicy::ApplyNonBreaking,
        SchemaChangePolicy::ApplyAll,
        SchemaChangePolicy::AskFirst,
        SchemaChangePolicy::Pause,
    ];

    // ---- diff -------------------------------------------------------------

    /// A first observation is the baseline: nothing to compare with.
    #[test]
    fn a_first_observation_has_no_changes() {
        assert!(diff(None, &base()).is_empty());
        for policy in POLICIES {
            let d = evaluate(policy, true, None, &base());
            assert_eq!(d.action, Action::Load);
            assert_eq!(d.columns, None);
            assert!(!d.pause_connector);
            assert!(d.changes.is_empty());
        }
    }

    #[test]
    fn the_same_shape_has_no_changes() {
        assert!(diff(Some(&base()), &base()).is_empty());
    }

    /// Spec row "Detection": added.
    #[test]
    fn an_added_column_is_found() {
        let observed = shape(
            &[("id", "integer"), ("note", "text"), ("qty", "integer")],
            &["id"],
        );
        assert_eq!(
            diff(Some(&base()), &observed),
            vec![Change::ColumnAdded {
                name: "qty".to_owned(),
                type_name: "integer".to_owned()
            }]
        );
    }

    /// Spec row "Detection": removed.
    #[test]
    fn a_removed_column_is_found() {
        let observed = shape(&[("id", "integer")], &["id"]);
        assert_eq!(
            diff(Some(&base()), &observed),
            vec![Change::ColumnRemoved {
                name: "note".to_owned(),
                type_name: "text".to_owned()
            }]
        );
    }

    /// Spec row "Detection": a rename is one removal and one addition.
    #[test]
    fn a_rename_is_a_removal_plus_an_addition() {
        let observed = shape(&[("id", "integer"), ("remark", "text")], &["id"]);
        let changes = diff(Some(&base()), &observed);
        assert_eq!(changes.len(), 2);
        assert!(matches!(&changes[0], Change::ColumnAdded { name, .. } if name == "remark"));
        assert!(matches!(&changes[1], Change::ColumnRemoved { name, .. } if name == "note"));
    }

    /// Names are compared exactly: `Note` is not `note`.
    #[test]
    fn column_names_are_compared_case_sensitively() {
        let observed = shape(&[("id", "integer"), ("Note", "text")], &["id"]);
        assert_eq!(diff(Some(&base()), &observed).len(), 2);
    }

    /// Spec row "Detection": type changed.
    #[test]
    fn a_type_change_is_found_with_its_class() {
        let observed = shape(&[("id", "bigint"), ("note", "text")], &["id"]);
        assert_eq!(
            diff(Some(&base()), &observed),
            vec![Change::TypeChanged {
                name: "id".to_owned(),
                before: "integer".to_owned(),
                after: "bigint".to_owned(),
                change: TypeChange::Widened
            }]
        );
    }

    #[test]
    fn a_changed_primary_key_is_found_and_its_order_is_not_a_change() {
        let two = shape(&[("a", "integer"), ("b", "integer")], &["a", "b"]);
        let swapped = shape(&[("a", "integer"), ("b", "integer")], &["b", "a"]);
        assert!(diff(Some(&two), &swapped).is_empty());
        let other = shape(&[("a", "integer"), ("b", "integer")], &["a"]);
        let changes = diff(Some(&two), &other);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].is_breaking());
    }

    // ---- type classification ----------------------------------------------

    fn widened(a: &str, b: &str) {
        assert_eq!(classify_type(a, b), Some(TypeChange::Widened), "{a} -> {b}");
        assert_eq!(
            classify_type(b, a),
            Some(TypeChange::Narrowed),
            "{b} -> {a}"
        );
    }

    #[test]
    fn integers_widen_up_the_family_in_every_spelling() {
        widened("smallint", "integer");
        widened("int2", "int4");
        widened("integer", "bigint");
        widened("int", "int8");
        widened("tinyint", "smallint");
        widened("smallint", "mediumint");
        widened("mediumint", "int");
        widened("tinyint", "bigint");
    }

    #[test]
    fn an_integer_display_width_is_not_a_change() {
        assert_eq!(classify_type("int(11)", "integer"), None);
        assert_eq!(classify_type("INT", "int4"), None);
    }

    #[test]
    fn a_float_widens_to_a_double() {
        widened("real", "double precision");
        widened("float4", "float8");
        widened("real", "double");
        assert_eq!(classify_type("Double  Precision", "float8"), None);
    }

    #[test]
    fn a_longer_text_widens_and_a_shorter_one_narrows() {
        widened("varchar(40)", "varchar(80)");
        widened("char(10)", "char(20)");
        widened("nvarchar(10)", "nvarchar(255)");
        widened("character varying(40)", "varchar(80)");
        widened("varchar(40)", "char(80)");
    }

    #[test]
    fn bounded_text_widens_to_unbounded_text() {
        widened("varchar(40)", "text");
        widened("char(10)", "varchar");
        widened("nvarchar(40)", "nvarchar(max)");
        assert_eq!(classify_type("text", "varchar"), None);
    }

    #[test]
    fn the_same_text_length_under_another_spelling_is_other() {
        assert_eq!(
            classify_type("char(10)", "varchar(10)"),
            Some(TypeChange::Other)
        );
    }

    /// `SRC-8 review BLOCKER 1`: replaces `more_decimal_digits_widen` and
    /// `decimal_digits_moving_in_opposite_directions_are_other`. Any change
    /// of a decimal's precision or scale is breaking; the same type in
    /// another spelling is still no change.
    #[test]
    fn any_change_to_a_decimals_precision_or_scale_is_other() {
        for (a, b) in [
            ("numeric(10,2)", "numeric(12,2)"),
            ("numeric(12,2)", "numeric(10,2)"),
            ("numeric(10,2)", "numeric(10,4)"),
            ("decimal(10,2)", "decimal(10,4)"),
            ("numeric(10)", "numeric(12)"),
            ("numeric(10,2)", "numeric"),
            ("numeric(10,4)", "numeric(12,2)"),
        ] {
            assert_eq!(classify_type(a, b), Some(TypeChange::Other), "{a} -> {b}");
        }
        assert_eq!(classify_type("numeric(10, 2)", "decimal(10,2)"), None);
    }

    /// `SRC-8 review BLOCKER 1`: replaces
    /// `anything_widens_to_text_and_text_back_is_narrowed`. A non-text type
    /// becoming text, and text becoming a non-text type, cannot be loaded
    /// into the existing column.
    #[test]
    fn a_non_text_type_becoming_text_is_other_and_so_is_text_becoming_a_number() {
        for (a, b) in [
            ("integer", "text"),
            ("text", "integer"),
            ("integer", "varchar(20)"),
            ("varchar(20)", "integer"),
            ("timestamp", "text"),
            ("numeric(10,2)", "varchar"),
            ("boolean", "longtext"),
            ("longtext", "boolean"),
        ] {
            assert_eq!(classify_type(a, b), Some(TypeChange::Other), "{a} -> {b}");
        }
    }

    #[test]
    fn a_change_on_neither_list_is_other() {
        for (a, b) in [
            ("integer", "numeric(10,2)"),
            ("integer", "varchar(10)"),
            ("timestamp", "date"),
            ("double precision", "integer"),
            ("int unsigned", "int"),
            ("varchar(40)[]", "varchar(80)[]"),
        ] {
            assert_eq!(classify_type(a, b), Some(TypeChange::Other), "{a} -> {b}");
        }
    }

    #[test]
    fn an_unchanged_type_in_another_case_or_spacing_is_no_change() {
        assert_eq!(classify_type("VARCHAR(40)", "varchar( 40 )"), None);
        assert_eq!(
            classify_type("timestamp without time zone", "Timestamp Without Time Zone"),
            None
        );
    }

    // ---- decide: the spec's Target table, per policy ----------------------

    fn with_added() -> TableShape {
        shape(
            &[("id", "integer"), ("note", "text"), ("qty", "integer")],
            &["id"],
        )
    }

    fn with_removed() -> TableShape {
        shape(&[("id", "integer")], &["id"])
    }

    fn with_widened() -> TableShape {
        shape(&[("id", "bigint"), ("note", "text")], &["id"])
    }

    fn statuses(d: &Decision) -> Vec<ChangeStatus> {
        d.changes.iter().map(|c| c.status).collect()
    }

    /// Spec "Per-connector policy", default and "apply all": a non-breaking
    /// change is applied, the table loads everything.
    #[test]
    fn apply_non_breaking_and_apply_all_apply_an_added_column_and_a_widened_type() {
        for policy in [
            SchemaChangePolicy::ApplyNonBreaking,
            SchemaChangePolicy::ApplyAll,
        ] {
            for observed in [with_added(), with_widened()] {
                let d = evaluate(policy, true, Some(&base()), &observed);
                assert_eq!(d.action, Action::Load);
                assert_eq!(d.columns, None);
                assert!(!d.pause_connector);
                assert_eq!(statuses(&d), vec![ChangeStatus::Applied]);
            }
        }
    }

    /// Spec "ask first": listed, not applied; the previously accepted
    /// columns that still exist keep loading.
    #[test]
    fn ask_first_holds_a_non_breaking_change_and_loads_the_known_columns() {
        let d = evaluate(
            SchemaChangePolicy::AskFirst,
            true,
            Some(&base()),
            &with_added(),
        );
        assert_eq!(d.action, Action::Load);
        assert_eq!(d.columns, Some(vec!["id".to_owned(), "note".to_owned()]));
        assert!(!d.pause_connector);
        assert_eq!(statuses(&d), vec![ChangeStatus::Pending]);
    }

    /// Spec "pause": any change stops the load and pauses the connector.
    #[test]
    fn pause_waits_pauses_the_connector_and_holds_the_change() {
        let d = evaluate(
            SchemaChangePolicy::Pause,
            true,
            Some(&base()),
            &with_added(),
        );
        assert_eq!(d.action, Action::Wait);
        assert!(d.pause_connector);
        assert_eq!(statuses(&d), vec![ChangeStatus::Pending]);
    }

    /// Spec "Breaking changes": a removed column, a narrowed type, an
    /// unclassified type and a changed key wait under every policy.
    #[test]
    fn a_breaking_change_waits_under_every_policy() {
        let narrowed = shape(&[("id", "smallint"), ("note", "text")], &["id"]);
        let other = shape(&[("id", "timestamp"), ("note", "text")], &["id"]);
        let rekeyed = shape(&[("id", "integer"), ("note", "text")], &["note"]);
        for observed in [with_removed(), narrowed, other, rekeyed] {
            for policy in POLICIES {
                let d = evaluate(policy, true, Some(&base()), &observed);
                assert_eq!(d.action, Action::Wait, "{policy:?}");
                assert_eq!(d.columns, None);
                assert!(d.changes.iter().any(|c| c.change.is_breaking()));
                assert!(d.changes.iter().all(|c| c.status == ChangeStatus::Pending));
                assert_eq!(d.pause_connector, policy == SchemaChangePolicy::Pause);
            }
        }
    }

    /// A breaking change and an added column together: the whole table
    /// waits, so approval can accept the observed shape as one.
    #[test]
    fn a_breaking_change_with_an_added_column_holds_both() {
        let observed = shape(&[("id", "integer"), ("qty", "integer")], &["id"]);
        let d = evaluate(SchemaChangePolicy::ApplyAll, true, Some(&base()), &observed);
        assert_eq!(d.action, Action::Wait);
        assert_eq!(d.changes.len(), 2);
        assert!(d.changes.iter().all(|c| c.status == ChangeStatus::Pending));
    }

    /// The property in words: while the source can hold back, a breaking
    /// change never answers "load" (and so never "load every column").
    #[test]
    fn a_breaking_change_never_loads_the_full_column_list_when_the_source_can_hold_back() {
        let observeds = [
            with_removed(),
            shape(&[("id", "smallint"), ("note", "text")], &["id"]),
            shape(&[("id", "date"), ("note", "text")], &["id"]),
            shape(&[("id", "integer"), ("note", "text")], &["note"]),
        ];
        for observed in &observeds {
            for policy in POLICIES {
                let d = evaluate(policy, true, Some(&base()), observed);
                assert_ne!(d.action, Action::Load, "{policy:?} {observed:?}");
            }
        }
    }

    /// Decision D4: where the source cannot hold back, the changes are
    /// already in the table. Recorded applied, always `Load`; `pause` still
    /// stops later runs.
    #[test]
    fn a_source_that_cannot_hold_back_always_loads_and_records_applied() {
        for observed in [with_added(), with_removed(), with_widened()] {
            for policy in POLICIES {
                let d = evaluate(policy, false, Some(&base()), &observed);
                assert_eq!(d.action, Action::Load);
                assert_eq!(d.columns, None);
                assert!(d.changes.iter().all(|c| c.status == ChangeStatus::Applied));
                assert_eq!(d.pause_connector, policy == SchemaChangePolicy::Pause);
            }
        }
    }

    /// The stored row carries names only, and `breaking` follows the class.
    #[test]
    fn a_change_becomes_a_row_of_names_and_types() {
        let d = evaluate(
            SchemaChangePolicy::ApplyAll,
            true,
            Some(&base()),
            &with_widened(),
        );
        let row = d.changes[0].to_new_change();
        assert_eq!(row.kind, ChangeKind::TypeChanged);
        assert_eq!(row.column_name, "id");
        assert_eq!(row.before_value.as_deref(), Some("integer"));
        assert_eq!(row.after_value.as_deref(), Some("bigint"));
        assert!(!row.breaking);
        let key = Change::PrimaryKeyChanged {
            before: vec!["a".to_owned(), "b".to_owned()],
            after: vec!["a".to_owned()],
        };
        let row = ClassifiedChange {
            change: key,
            status: ChangeStatus::Pending,
        }
        .to_new_change();
        assert_eq!(row.column_name, "");
        assert_eq!(row.before_value.as_deref(), Some("a, b"));
    }

    /// `SRC-8 review BLOCKER 3a`: under `pause`, a type change the column
    /// cannot hold makes the table wait but never pauses the connector, so a
    /// later run can see the column put back; every other policy is as before.
    #[test]
    fn a_type_change_that_cannot_be_approved_never_pauses_the_connector() {
        let other = shape(&[("id", "timestamp"), ("note", "text")], &["id"]);
        // `Other` together with an added column: still not pausable.
        let other_and_added = shape(
            &[("id", "timestamp"), ("note", "text"), ("qty", "integer")],
            &["id"],
        );
        for observed in [other, other_and_added] {
            for policy in POLICIES {
                let d = evaluate(policy, true, Some(&base()), &observed);
                assert_eq!(d.action, Action::Wait, "{policy:?}");
                assert!(!d.pause_connector, "{policy:?}");
                assert!(d.changes.iter().all(|c| c.status == ChangeStatus::Pending));
            }
        }
    }

    /// The other breaking changes still pause under `pause`: a removed
    /// column, a narrowed type and a changed key can all be approved.
    #[test]
    fn a_breaking_change_that_can_be_approved_still_pauses_under_pause() {
        let narrowed = shape(&[("id", "smallint"), ("note", "text")], &["id"]);
        let rekeyed = shape(&[("id", "integer"), ("note", "text")], &["note"]);
        for observed in [with_removed(), narrowed, rekeyed] {
            let d = evaluate(SchemaChangePolicy::Pause, true, Some(&base()), &observed);
            assert_eq!(d.action, Action::Wait);
            assert!(d.pause_connector);
        }
    }
}
