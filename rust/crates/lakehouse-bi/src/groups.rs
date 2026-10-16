//! Value groups (`BI-10`, with `AI-6`): a saved field that gathers chosen
//! values of a text column under names of the user's own ("Java" for six
//! provinces), with the rest kept as they are or gathered under "Other".
//!
//! # Where it lives
//!
//! A group is stored with the calculated fields of its table or SQL source
//! (`console.bi_field`), with `level = "group"`, `data_type = "text"` and the
//! canonical JSON of [`GroupDef`] in `formula`. No new column, no migration.
//! A field read through [`crate::fields::FieldCatalog`] therefore lists, saves
//! and deletes like any other, with the same permissions, the same uniqueness
//! of the name and the same rule against deleting one in use.
//!
//! # Safety of the SQL
//!
//! A group is compiled by [`GroupDef::sql`] to `multiIf(toString(col) IN
//! (...), 'Name', ..., <rest>)`. The column is an [`Ident`] checked against the
//! source; every name and every value is printed through [`SqlLiteral`],
//! which escapes quotes, backslashes and control characters; nothing a person
//! typed is parsed as an expression. A value that is NULL matches no group
//! (`NULL IN (...)` is NULL, and `multiIf` skips a NULL condition), so it falls
//! to the rest: kept as NULL, or counted under "Other".
//!
//! # What it does not do
//!
//! A group is a per-row text column of its source. It cannot be named inside
//! another field's formula (the formula language reads columns and formula
//! fields, not groups), and groups over numbers or dates are not offered: bins
//! and grains cover those.

use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::{Deserialize, Serialize};

use crate::filters::ColumnKind;

/// The `level` word a stored group carries.
pub const LEVEL: &str = "group";

/// Most groups in one field, and most values in all of them.
pub const MAX_GROUPS: usize = 50;
/// See [`MAX_GROUPS`].
pub const MAX_VALUES: usize = 2000;
/// Longest group name, and longest value, in characters.
pub const MAX_NAME_LEN: usize = 64;
/// See [`MAX_NAME_LEN`].
pub const MAX_VALUE_LEN: usize = 256;

/// What the rest (values in no group) becomes.
pub const OTHER_LABEL: &str = "Other";

/// One named group of values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueGroup {
    /// The name the values are shown under.
    pub name: String,
    /// The values, as they are spelled in the column.
    pub values: Vec<String>,
}

/// A value group's definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupDef {
    /// The text column the groups read.
    pub column: String,
    /// The groups, in order.
    pub groups: Vec<ValueGroup>,
    /// `keep` (the rest keep their values) or `other` (the rest are "Other").
    pub rest: String,
}

impl GroupDef {
    /// The definition stored in a field: the canonical JSON text.
    #[must_use]
    pub fn to_stored(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Read a stored definition. `None` for text that is not one.
    #[must_use]
    pub fn from_stored(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    /// Check the definition against the source's columns.
    ///
    /// # Errors
    ///
    /// The sentence to show the person who defined it.
    pub fn validate<H: std::hash::BuildHasher>(
        &self,
        columns: &std::collections::HashMap<String, ColumnKind, H>,
    ) -> Result<(), String> {
        if Ident::new(self.column.as_str()).is_err() || !columns.contains_key(&self.column) {
            return Err(format!("'{}' is not a column of this source.", self.column));
        }
        if columns.get(&self.column) != Some(&ColumnKind::Text) {
            return Err(format!(
                "'{}' is not a text column; bins group numbers and a grain groups dates.",
                self.column
            ));
        }
        if self.rest != "keep" && self.rest != "other" {
            return Err("the rest is either kept or called 'Other'.".to_owned());
        }
        if self.groups.is_empty() || self.groups.len() > MAX_GROUPS {
            return Err(format!("a field has 1 to {MAX_GROUPS} groups."));
        }
        let mut names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut total = 0usize;
        for g in &self.groups {
            let name = g.name.trim();
            if name.is_empty()
                || name.chars().count() > MAX_NAME_LEN
                || name.chars().any(char::is_control)
            {
                return Err(format!(
                    "a group name is 1 to {MAX_NAME_LEN} characters, without control characters."
                ));
            }
            if name != g.name {
                return Err(format!("the group name '{}' has spaces at an end.", g.name));
            }
            if !names.insert(name) {
                return Err(format!("the group '{name}' is listed twice."));
            }
            if self.rest == "other" && name == OTHER_LABEL {
                return Err(
                    "a group cannot be called 'Other' while the rest is called 'Other'.".to_owned(),
                );
            }
            if g.values.is_empty() {
                return Err(format!("the group '{name}' has no values."));
            }
            for v in &g.values {
                total += 1;
                if v.chars().count() > MAX_VALUE_LEN {
                    return Err(format!("a value is at most {MAX_VALUE_LEN} characters."));
                }
                if !seen.insert(v.as_str()) {
                    return Err(format!(
                        "a value can be in one group only; one is repeated in '{name}'."
                    ));
                }
            }
        }
        if total > MAX_VALUES {
            return Err(format!("a field holds at most {MAX_VALUES} values in all."));
        }
        Ok(())
    }

    /// The per-row expression. Call [`Self::validate`] first.
    ///
    /// `None` when the column is not an identifier.
    #[must_use]
    pub fn sql(&self) -> Option<String> {
        let column = Ident::new(self.column.as_str()).ok()?;
        let mut parts: Vec<String> = Vec::new();
        for g in &self.groups {
            let list = g
                .values
                .iter()
                .map(|v| SqlLiteral::from(v.as_str()).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("toString({column}) IN ({list})"));
            parts.push(SqlLiteral::from(g.name.as_str()).to_string());
        }
        parts.push(if self.rest == "other" {
            SqlLiteral::from(OTHER_LABEL).to_string()
        } else {
            format!("toString({column})")
        });
        Some(format!("multiIf({})", parts.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;

    fn def(json: &str) -> GroupDef {
        serde_json::from_str(json).unwrap()
    }

    fn cols() -> HashMap<String, ColumnKind> {
        HashMap::from([
            ("prov".to_owned(), ColumnKind::Text),
            ("n".to_owned(), ColumnKind::Number),
        ])
    }

    const JAVA: &str = r#"{"column":"prov","rest":"other","groups":[{"name":"Java","values":["Jawa Barat","Jawa Timur"]},{"name":"Sumatra","values":["Aceh"]}]}"#;

    #[test]
    fn a_group_compiles_to_one_conditional_over_escaped_literals() {
        let d = def(JAVA);
        d.validate(&cols()).unwrap();
        assert_eq!(
            d.sql().unwrap(),
            "multiIf(toString(prov) IN ('Jawa Barat', 'Jawa Timur'), 'Java', toString(prov) IN ('Aceh'), 'Sumatra', 'Other')"
        );
        let keep = def(&JAVA.replace("other", "keep"));
        assert!(keep.sql().unwrap().ends_with(", toString(prov))"));
    }

    #[test]
    fn quotes_backslashes_and_non_ascii_come_out_only_inside_escaped_literals() {
        let hostile = [
            "O'Brien",
            "x') OR 1=1 --",
            "back\\slash",
            "tab\there",
            "ñandú — 日本",
            "'; DROP TABLE t; --",
        ];
        for v in hostile {
            for name in ["it's", "a\\b", "名前"] {
                let d = GroupDef {
                    column: "prov".to_owned(),
                    rest: "keep".to_owned(),
                    groups: vec![ValueGroup {
                        name: name.to_owned(),
                        values: vec![v.to_owned()],
                    }],
                };
                d.validate(&cols()).unwrap();
                let sql = d.sql().unwrap();
                // Every quote of a value or a name is escaped, so the text
                // between literals is only the fixed skeleton.
                let skeleton = strip_literals(&sql);
                assert_eq!(
                    skeleton, "multiIf(toString(prov) IN (), , toString(prov))",
                    "{v:?} {name:?}: {sql}"
                );
            }
        }
    }

    /// Remove every well-formed SQL string literal (`'...'` with `\'` and `\\`
    /// escapes) from `sql`; whatever is left is what the engine reads as code.
    fn strip_literals(sql: &str) -> String {
        let mut out = String::new();
        let mut chars = sql.chars();
        while let Some(c) = chars.next() {
            if c != '\'' {
                out.push(c);
                continue;
            }
            loop {
                match chars.next() {
                    Some('\\') => {
                        chars.next();
                    }
                    Some('\'') | None => break,
                    Some(_) => {}
                }
            }
        }
        out
    }

    #[test]
    fn a_definition_that_does_not_make_sense_is_refused_with_a_sentence() {
        let bad = |json: &str| def(json).validate(&cols()).unwrap_err();
        assert!(bad(&JAVA.replace("prov", "n")).contains("not a text column"));
        assert!(bad(&JAVA.replace("\"prov\"", "\"absent\"")).contains("not a column"));
        assert!(bad(&JAVA.replace("\"other\"", "\"x\"")).contains("rest"));
        assert!(bad(r#"{"column":"prov","rest":"keep","groups":[]}"#).contains("groups"));
        assert!(bad(&JAVA.replace("Aceh", "Jawa Barat")).contains("one group only"));
        assert!(bad(&JAVA.replace("Sumatra", "Java")).contains("twice"));
        assert!(bad(&JAVA.replace("\"Sumatra\"", "\"Other\"")).contains("Other"));
        assert!(bad(&JAVA.replace("\"Sumatra\"", "\" \"")).contains("name"));
    }

    #[test]
    fn the_limits_are_fifty_groups_and_two_thousand_values() {
        let groups: Vec<ValueGroup> = (0..51)
            .map(|i| ValueGroup {
                name: format!("g{i}"),
                values: vec![format!("v{i}")],
            })
            .collect();
        let d = GroupDef {
            column: "prov".to_owned(),
            rest: "keep".to_owned(),
            groups,
        };
        assert!(d.validate(&cols()).unwrap_err().contains("50"));
        let big = GroupDef {
            column: "prov".to_owned(),
            rest: "keep".to_owned(),
            groups: (0..21)
                .map(|g| ValueGroup {
                    name: format!("g{g}"),
                    values: (0..100).map(|v| format!("{g}-{v}")).collect(),
                })
                .collect(),
        };
        assert!(big.validate(&cols()).unwrap_err().contains("2000"));
    }

    #[test]
    fn a_definition_survives_storage() {
        let d = def(JAVA);
        assert_eq!(GroupDef::from_stored(&d.to_stored()), Some(d));
        assert_eq!(GroupDef::from_stored("Sum(x)"), None);
    }
}
